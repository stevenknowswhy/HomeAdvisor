use std::env;

use zeroize::Zeroizing;

use crate::StoreError;

/// The SQLCipher database key.
///
/// Deliberately carries no `Debug` or `Display` implementation: key material
/// must never be formattable into a log line. The bytes are zeroized on
/// drop. The key arrives at runtime from the environment or the OS keystore
/// and is never persisted beside the database.
pub struct StoreKey {
    secret: Zeroizing<String>,
}

impl StoreKey {
    /// Wrap key material already in hand — for example a passphrase read
    /// from the OS keystore by the platform shell.
    pub fn from_passphrase(key: impl Into<String>) -> Result<Self, StoreError> {
        let key = key.into();
        if key.is_empty() {
            return Err(StoreError::KeyEmpty);
        }
        Ok(Self {
            secret: Zeroizing::new(key),
        })
    }

    /// Read the key from the environment — the milestone-1 keystore
    /// hand-off path. The platform shell later swaps in native keystore
    /// readers (Keychain, DPAPI, Android Keystore) behind this same type.
    pub fn from_env(var: &str) -> Result<Self, StoreError> {
        match env::var(var) {
            Ok(key) => Self::from_passphrase(key),
            Err(env::VarError::NotPresent) => Err(StoreError::KeyEnvMissing {
                var: var.to_owned(),
            }),
            Err(env::VarError::NotUnicode(_)) => Err(StoreError::KeyEnvNotUnicode {
                var: var.to_owned(),
            }),
        }
    }

    pub(crate) fn passphrase(&self) -> &str {
        self.secret.as_str()
    }
}
