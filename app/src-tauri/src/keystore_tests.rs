//! The startup key machine, state by state, against an in-memory keystore
//! double — no macOS, no Keychain, no network. The real Keychain round-trip
//! is `the_real_login_keychain_round_trips` below; it compiles (and runs in
//! CI, `tauri-build-macos`) only on macOS.

use std::cell::RefCell;
use std::path::Path;

use ha_store::{Store, StoreKey};

use crate::keystore::{
    generate_key, resolve_startup_key, to_hex, KeyStore, KeyStoreError, StartupKeyError,
};

/// A valid override the tests can rely on: 32 bytes of entropy as hex.
const TEST_KEY: &str = "5f3a9c1e7b2d48a06c3e91f4d27b58a03e6c19d4f8a27b5c3e9146d0f8a3b57c";

/// The in-memory double: `empty` models a machine with no key, `stocked` a
/// machine whose key is filed away, `broken` a keychain that refuses reads.
/// The call counters pin the *absence* of side effects where it matters.
struct MemoryKeyStore {
    item: RefCell<Option<String>>,
    reads_fail: bool,
    reads: RefCell<u32>,
    adds: RefCell<u32>,
}

impl MemoryKeyStore {
    fn empty() -> Self {
        Self {
            item: RefCell::new(None),
            reads_fail: false,
            reads: RefCell::new(0),
            adds: RefCell::new(0),
        }
    }

    fn stocked(key: &str) -> Self {
        Self {
            item: RefCell::new(Some(key.to_owned())),
            reads_fail: false,
            reads: RefCell::new(0),
            adds: RefCell::new(0),
        }
    }

    fn broken() -> Self {
        Self {
            reads_fail: true,
            ..Self::empty()
        }
    }
}

impl KeyStore for MemoryKeyStore {
    fn read(&self) -> Result<Option<String>, KeyStoreError> {
        *self.reads.borrow_mut() += 1;
        if self.reads_fail {
            return Err(KeyStoreError::new("read", "the keychain is locked"));
        }
        Ok(self.item.borrow().clone())
    }

    fn add(&self, key: &str) -> Result<(), KeyStoreError> {
        *self.adds.borrow_mut() += 1;
        *self.item.borrow_mut() = Some(key.to_owned());
        Ok(())
    }
}

/// The generator tests use instead of the real one: valid hex, no entropy,
/// no surprise.
fn fixed_key() -> Result<String, KeyStoreError> {
    Ok(TEST_KEY.to_owned())
}

/// `expect_err` for results whose Ok type deliberately carries no `Debug`:
/// `StoreKey` refuses to be formattable — key material must never reach a
/// log line, including a panic message.
fn expect_key_error(result: Result<StoreKey, StartupKeyError>, why: &str) -> StartupKeyError {
    match result {
        Ok(_) => panic!("{why}: resolved a key instead"),
        Err(error) => error,
    }
}

/// A private on-disk path for the one test that must prove key equality
/// cryptographically (SQLCipher only opens a store with the very key that
/// encrypted it — in-memory stores cannot carry that proof across drops).
fn on_disk_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ha-keystore-test-{}-{name}", std::process::id()))
}

#[test]
fn fresh_create_generates_stores_and_returns_a_key() {
    let keystore = MemoryKeyStore::empty();
    let db_path = on_disk_path("fresh-create.db");

    let key = resolve_startup_key(&db_path, false, None, Some(&keystore), fixed_key)
        .expect("fresh create must resolve a key");

    // The key was filed away for the next launch, and it is the generated
    // one — not invented elsewhere.
    assert_eq!(*keystore.adds.borrow(), 1);
    assert_eq!(keystore.read().unwrap(), Some(TEST_KEY.to_owned()));
    // The resolved key actually opens a SQLCipher store.
    Store::open_in_memory(&key).expect("the resolved key opens a store");
}

#[test]
fn reopen_uses_the_stored_key_without_storing_a_new_one() {
    let keystore = MemoryKeyStore::stocked(TEST_KEY);
    let db_path = on_disk_path("reopen.db");

    // The store this (simulated) earlier run encrypted.
    let seeded_key = StoreKey::from_passphrase(TEST_KEY).unwrap();
    Store::open(&db_path, &seeded_key).expect("seed run creates the store");
    drop(seeded_key);

    let key = resolve_startup_key(&db_path, true, None, Some(&keystore), fixed_key)
        .expect("reopen must resolve the stored key");

    // No regeneration on reopen — the stored key is the only one that can
    // open the store, and SQLCipher proves equality by opening it.
    assert_eq!(*keystore.adds.borrow(), 0);
    Store::open(&db_path, &key).expect("the resolved key reopens the existing store");
    std::fs::remove_file(&db_path).ok();
}

#[test]
fn missing_key_fails_closed_and_never_writes() {
    let keystore = MemoryKeyStore::empty();
    let db_path = on_disk_path("missing-key.db");

    let error = expect_key_error(
        resolve_startup_key(&db_path, true, None, Some(&keystore), fixed_key),
        "a database without its key must fail closed",
    );

    assert!(matches!(error, StartupKeyError::MissingKey { .. }));
    // The failure does not paper over itself with a fresh key: generating
    // one over existing data would quietly brick the store.
    assert_eq!(*keystore.adds.borrow(), 0);
    // The diagnostic is the product: readable, and it names the database.
    let diagnostic = error.to_string();
    assert!(diagnostic.contains("cannot unlock"), "{diagnostic}");
    assert!(
        diagnostic.contains(&format!("{}", db_path.display())),
        "{diagnostic}"
    );
}

#[test]
fn an_unreadable_keystore_fails_closed() {
    let keystore = MemoryKeyStore::broken();
    let db_path = on_disk_path("unreadable.db");

    let error = expect_key_error(
        resolve_startup_key(&db_path, true, None, Some(&keystore), fixed_key),
        "a broken keychain must fail closed",
    );

    assert!(matches!(error, StartupKeyError::UnreadableKey { .. }));
    assert_eq!(*keystore.adds.borrow(), 0);
}

#[test]
fn the_env_override_wins_without_touching_the_keystore() {
    // Even a broken keychain cannot interfere with the override — the
    // milestone-1 hand-off that tests and the CLI interop depend on.
    let keystore = MemoryKeyStore::broken();
    let db_path = on_disk_path("override.db");

    let key = resolve_startup_key(
        &db_path,
        true,
        Some(TEST_KEY.to_owned()),
        Some(&keystore),
        fixed_key,
    )
    .expect("the override wins");

    assert_eq!(*keystore.reads.borrow(), 0);
    Store::open_in_memory(&key).expect("the override key opens a store");
}

#[test]
fn an_empty_override_fails_with_a_usable_diagnostic() {
    let keystore = MemoryKeyStore::empty();

    let error = expect_key_error(
        resolve_startup_key(
            Path::new("unused.db"),
            false,
            Some(String::new()),
            Some(&keystore),
            fixed_key,
        ),
        "an empty override has always been an error",
    );

    assert!(matches!(error, StartupKeyError::InvalidOverride { .. }));
    assert!(error.to_string().contains("HOMEADVISOR_DB_KEY"));
}

#[test]
fn without_a_keystore_startup_requires_the_override() {
    // The non-macOS contract, with and without an existing database: no
    // keystore integrated, so the override is the only key path.
    for db_exists in [false, true] {
        let error = expect_key_error(
            resolve_startup_key(Path::new("unused.db"), db_exists, None, None, fixed_key),
            "no keystore and no override must fail closed",
        );

        assert!(matches!(error, StartupKeyError::NoKeyStore { .. }));
        assert!(error.to_string().contains("HOMEADVISOR_DB_KEY"));
    }
}

#[test]
fn generated_keys_are_64_lowercase_hex() {
    let first = generate_key().expect("the OS entropy source works");
    let second = generate_key().expect("the OS entropy source keeps working");

    assert_eq!(first.len(), 64);
    assert!(first
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    // Entropy sanity: two draws never agree.
    assert_ne!(first, second);
}

#[test]
fn to_hex_matches_a_known_vector() {
    assert_eq!(to_hex(&[0x00, 0xff, 0x0a, 0x1b]), "00ff0a1b");
    assert_eq!(to_hex(&[]), "");
}

/// The only test that talks to a real login Keychain — hence macOS-only,
/// and the CI step `tauri-build-macos` runs it by name (`keychain`). It also
/// proves the `-25300` item-not-found mapping in `read`.
#[cfg(target_os = "macos")]
#[test]
fn the_real_login_keychain_round_trips() {
    use crate::keystore::keychain::{KeychainKeyStore, SERVICE};

    // A previous run's item must not leak into this one: start clean.
    security_framework::passwords::delete_generic_password(SERVICE, "db-key").ok();

    let keystore = KeychainKeyStore;
    assert_eq!(keystore.read().unwrap(), None, "fresh start has no item");

    keystore.add(TEST_KEY).unwrap();
    assert_eq!(keystore.read().unwrap(), Some(TEST_KEY.to_owned()));

    // A second add updates the one item rather than duplicating it.
    keystore.add("second-value").unwrap();
    assert_eq!(keystore.read().unwrap(), Some("second-value".to_owned()));

    security_framework::passwords::delete_generic_password(SERVICE, "db-key").unwrap();
    assert_eq!(keystore.read().unwrap(), None, "the item is gone again");
}
