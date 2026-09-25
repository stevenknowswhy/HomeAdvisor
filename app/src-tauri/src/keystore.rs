//! Startup database-key management: where the encrypted store's key comes
//! from, and what happens when it cannot be found.
//!
//! Every launch resolves the key through one of four states:
//!
//! 1. **Override** — `HOMEADVISOR_DB_KEY` is set: use it verbatim. The
//!    milestone-1 hand-off; tests and the CLI interop rely on it unchanged.
//! 2. **Reopen** — the platform keystore holds the key: read and use it.
//! 3. **Fresh create** — no stored key and no database yet: generate 256
//!    bits and file it in the platform keystore *before* anything touches
//!    disk, so a database is never created this machine could not reopen.
//!    On macOS the keystore is the login Keychain (generic password, service
//!    `com.forhemit.homeadvisor`, account `db-key`); no other platform has a
//!    keystore integrated yet.
//! 4. **Fail closed** — a database exists but its key is missing or
//!    unreadable (or there is no keystore to hold a new one): a
//!    plain-language diagnostic and a clean nonzero exit. The store is
//!    ciphertext without its key; opening "something" would be a lie, and
//!    regenerating a key over existing data would quietly brick it.
//!
//! [`crate::state::open_state`] runs this machine, and `lib.rs` calls that
//! *before* the event loop: on macOS the Tauri setup hook runs inside
//! `did_finish_launching`, where a returned error unwinds across the
//! Objective-C boundary and aborts the process before any window exists —
//! the v0.1.0 first-run SIGABRT every fresh user hit. Before the loop, the
//! same failure is an `eprintln!` and `exit(1)` instead.

use std::path::{Path, PathBuf};

use ha_store::StoreKey;

/// Reads and writes the database key in an OS keystore. Narrow on purpose —
/// `add` and `read` are the whole surface, so the in-memory double in
/// `keystore_tests.rs` is the entire test harness for the state machine.
pub(crate) trait KeyStore {
    /// The stored key, or `None` when this machine holds none.
    fn read(&self) -> Result<Option<String>, KeyStoreError>;
    /// Store `key`, replacing any item already there.
    fn add(&self, key: &str) -> Result<(), KeyStoreError>;
}

/// A keystore operation failed. `operation` names the half that broke and
/// `detail` carries the plain cause that reaches the startup diagnostic.
#[derive(Debug, thiserror::Error)]
#[error("keystore {operation} failed: {detail}")]
pub(crate) struct KeyStoreError {
    pub(crate) operation: &'static str,
    pub(crate) detail: String,
}

impl KeyStoreError {
    /// Constructed by the platform impls and by the test double.
    pub(crate) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

/// Why key resolution failed, in words a user can act on. Rendered verbatim
/// by the startup diagnostic in `lib.rs` — never a bare panic or a raw error
/// code.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StartupKeyError {
    /// The override is set but not usable. The env-var contract (and its
    /// error paths) is unchanged from `StoreKey::from_env`.
    #[error(
        "HOMEADVISOR_DB_KEY is set but not usable: {detail}\n\
         Set it to a non-empty passphrase, or unset it to let the app manage\n\
         the key itself."
    )]
    InvalidOverride { detail: String },

    /// No platform keystore on this OS and no override. The documented
    /// non-macOS startup contract (README, "The desktop app's database
    /// key"): the env variable is the only key path there.
    #[error(
        "Home Advisor has no database key.\n\
         On this platform the app starts only with HOMEADVISOR_DB_KEY set in\n\
         the environment (see the README); there is no OS keystore integration.\n\
         The encrypted database at {db_path} will not be opened without a key."
    )]
    NoKeyStore { db_path: PathBuf },

    /// The encrypted database exists but its key does not. The one
    /// unrecoverable state: without the key the store is ciphertext, and no
    /// recovery path exists on purpose.
    #[error(
        "Home Advisor cannot unlock its encrypted database.\n\
         The database at {db_path} is encrypted, and its key is missing from\n\
         the macOS login Keychain (service \"com.forhemit.homeadvisor\", account\n\
         \"db-key\"). Without that key the data cannot be decrypted — by the app\n\
         or by anyone else. To start over, delete the database file above and\n\
         launch again; this erases all stored data."
    )]
    MissingKey { db_path: PathBuf },

    /// The Keychain could not be read at all (locked, denied, clobbered
    /// item) while a database exists. The key may still be behind the
    /// failure, so the remedy is repair, not "start over".
    #[error(
        "Home Advisor could not read the database key from the macOS login\n\
         Keychain: {detail}\n\
         The encrypted database at {db_path} cannot be opened without it. If\n\
         the keychain is locked, unlock it and launch again."
    )]
    UnreadableKey { db_path: PathBuf, detail: String },

    /// A generated key could not be stored. Better to fail now than to
    /// create a database this machine could never reopen.
    #[error(
        "Home Advisor could not store a new database key in the macOS login\n\
         Keychain: {detail}\n\
         It will not create a database this machine could not reopen. Fix the\n\
         keychain problem and launch again."
    )]
    UnwritableKey { detail: String },

    /// The OS entropy source failed. Practically unreachable, but failing
    /// closed beats inventing a key.
    #[error("Home Advisor could not generate a database key: {detail}")]
    KeyGeneration { detail: String },
}

/// 256 bits of entropy — SQLCipher gets the hex form as its passphrase.
const KEY_HEX_CHARS: usize = 64;

/// Fresh key material as the hex string the keystore files and
/// `StoreKey::from_passphrase` consumes. Randomness comes from the OS via
/// `getrandom` — never from anything derived or user-chosen.
pub(crate) fn generate_key() -> Result<String, KeyStoreError> {
    let mut bytes = [0u8; KEY_HEX_CHARS / 2];
    getrandom::fill(&mut bytes)
        .map_err(|error| KeyStoreError::new("key generation", error.to_string()))?;
    Ok(to_hex(&bytes))
}

/// Lowercase hex — the storage and hand-off form of a generated key.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// The platform keystore, chosen at compile time. `None` where none is
/// integrated: startup then requires `HOMEADVISOR_DB_KEY` (documented in the
/// README, and enforced fail-closed by [`resolve_startup_key`]).
#[cfg(target_os = "macos")]
pub(crate) fn platform_keystore() -> Option<Box<dyn KeyStore>> {
    Some(Box::new(keychain::KeychainKeyStore))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn platform_keystore() -> Option<Box<dyn KeyStore>> {
    None
}

/// Resolve the startup key — the state machine in the module docs. Pure over
/// its inputs: the keystore and the fresh-key generator are parameters, so
/// every state is exercised with in-memory doubles, without macOS.
///
/// `env_key` is the pre-read `HOMEADVISOR_DB_KEY` (`None` = unset). When set
/// it wins outright — the keystore is not even consulted.
pub(crate) fn resolve_startup_key(
    db_path: &Path,
    db_exists: bool,
    env_key: Option<String>,
    keystore: Option<&dyn KeyStore>,
    generate_key: fn() -> Result<String, KeyStoreError>,
) -> Result<StoreKey, StartupKeyError> {
    // 1. The override wins, verbatim. Empty and non-unicode values fail with
    //    a plain-language version of the `from_env` errors they always raised.
    if let Some(key) = env_key {
        return StoreKey::from_passphrase(key).map_err(|error| StartupKeyError::InvalidOverride {
            detail: error.to_string(),
        });
    }

    let Some(keystore) = keystore else {
        // 4 (no keystore): the non-macOS contract.
        return Err(StartupKeyError::NoKeyStore {
            db_path: db_path.to_path_buf(),
        });
    };

    match keystore.read() {
        // 2. Reopen with the key this machine filed away.
        Ok(Some(stored)) => StoreKey::from_passphrase(stored).map_err(|error| {
            // The only failure `from_passphrase` raises is emptiness — a
            // clobbered item is unreadable in effect, with the same remedy.
            StartupKeyError::UnreadableKey {
                db_path: db_path.to_path_buf(),
                detail: error.to_string(),
            }
        }),
        // 3. Fresh create: no key on this machine and no database either.
        //    Generate first, file it, then hand it to the store.
        Ok(None) if !db_exists => {
            let key = generate_key().map_err(|error| StartupKeyError::KeyGeneration {
                detail: error.to_string(),
            })?;
            keystore
                .add(&key)
                .map_err(|error| StartupKeyError::UnwritableKey {
                    detail: error.to_string(),
                })?;
            StoreKey::from_passphrase(key).map_err(|error| StartupKeyError::UnreadableKey {
                db_path: db_path.to_path_buf(),
                detail: error.to_string(),
            })
        }
        // 4. Fail closed: the database exists but its key does not. No
        //    regeneration — that would quietly brick the data it claims to
        //    help.
        Ok(None) => Err(StartupKeyError::MissingKey {
            db_path: db_path.to_path_buf(),
        }),
        // 4. The keystore itself is broken. The key may exist behind the
        //    failure, so repair — not "start over" — is the remedy.
        Err(error) => Err(StartupKeyError::UnreadableKey {
            db_path: db_path.to_path_buf(),
            detail: error.to_string(),
        }),
    }
}

/// The macOS login Keychain behind Apple's Security.framework.
#[cfg(target_os = "macos")]
pub(crate) mod keychain {
    use security_framework::passwords::{get_generic_password, set_generic_password};

    use super::{KeyStore, KeyStoreError};

    /// The Keychain service and account the database key is filed under.
    /// Stable names — changing either orphans every installed key. The
    /// fail-closed diagnostics in [`super::StartupKeyError`] mirror these
    /// strings; keep them in sync.
    pub(crate) const SERVICE: &str = "com.forhemit.homeadvisor";
    pub(crate) const ACCOUNT: &str = "db-key";

    /// Apple's `errSecItemNotFound` (-25300): no item for this service and
    /// account. The crate exports no constant for it, and the numeric form
    /// is proven against the real Keychain by the macOS CI round-trip test
    /// (`the_real_login_keychain_round_trips`).
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    /// The login Keychain keystore. A unit struct: the item's coordinates
    /// are the constants above.
    pub(crate) struct KeychainKeyStore;

    impl KeyStore for KeychainKeyStore {
        fn read(&self) -> Result<Option<String>, KeyStoreError> {
            match get_generic_password(SERVICE, ACCOUNT) {
                // A machine with no key yet is a state, not an error.
                Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
                Ok(bytes) => String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|error| KeyStoreError::new("read", error.to_string())),
                Err(error) => Err(KeyStoreError::new("read", error.to_string())),
            }
        }

        fn add(&self, key: &str) -> Result<(), KeyStoreError> {
            set_generic_password(SERVICE, ACCOUNT, key.as_bytes())
                .map_err(|error| KeyStoreError::new("write", error.to_string()))
        }
    }
}
