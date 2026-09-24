//! The CLI's error surface. Every variant is user-facing: the demo either
//! completes and prints its screen, or stops with one clear message.

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("store error: {0}")]
    Store(#[from] ha_store::StoreError),

    #[error("privacy gate error: {0}")]
    Privacy(#[from] ha_privacy::PrivacyError),

    #[error("sidecar configuration error: {0}")]
    LayaConfig(#[from] ha_privacy::LayaConfigError),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// A demo-data problem: the fixture and the seeded taxonomy drifted
    /// apart, or a command ran against a database with no seeded family.
    #[error("{0}")]
    Demo(String),

    /// The invocation itself is wrong. Carries the usage text to print.
    #[error("{0}")]
    Usage(String),
}
