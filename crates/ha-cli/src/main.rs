//! The `ha-cli` binary — a thin dispatch over the library.
//!
//! No command logic lives here: the integration tests exercise the same
//! `args` → `commands` path this binary prints. The SQLCipher key never
//! comes from a file or a default; the caller supplies it per invocation
//! through `HA_STORE_KEY` (see `args::USAGE`) and the store wipes it from
//! memory when the process ends.

use std::process::ExitCode;

use ha_cli::{args, commands, error::CliError};
use ha_store::StoreKey;

/// The environment variable carrying the SQLCipher passphrase for one
/// invocation. Reading from the environment (not argv) keeps the key out
/// of process listings.
const KEY_ENV: &str = "HA_STORE_KEY";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            if let CliError::Usage(usage) = &error {
                eprintln!();
                eprintln!("{usage}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), CliError> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match args::parse(&raw)? {
        args::Parsed::Help => {
            println!("{}", args::USAGE);
            Ok(())
        }
        args::Parsed::Command(command) => dispatch(command),
    }
}

fn dispatch(command: args::Command) -> Result<(), CliError> {
    let key = store_key()?;

    match command {
        args::Command::Seed { db } => {
            let summary = commands::seed(&db, &key)?;
            println!(
                "{}",
                ha_cli::render::seed_screen(
                    &summary.db_display,
                    &summary.household_id,
                    summary.member_count,
                    summary.goal_count,
                    &summary.region_class,
                    &summary.income_band,
                )
            );
        }
        args::Command::Research { db, sidecar_url } => {
            let report = commands::research(&db, &key, &sidecar_url)?;
            println!("{}", report.rendered);
            if let ha_privacy::GateVerdict::Blocked(_) = report.outcome.verdict {
                // Blocked egress is a privacy success but a failed
                // request: the exit code says so for scripts.
                return Err(CliError::Demo(
                    "the request was blocked at the gate — see the receipt above and `ha-cli receipt --db <path>`"
                        .to_string(),
                ));
            }
        }
        args::Command::Receipt { db } => {
            let report = commands::receipt_log(&db, &key)?;
            println!("{}", report.rendered);
        }
    }
    Ok(())
}

fn store_key() -> Result<StoreKey, CliError> {
    let passphrase = std::env::var(KEY_ENV).map_err(|_| {
        CliError::Usage(format!(
            "the {KEY_ENV} environment variable must hold the database key for this invocation"
        ))
    })?;
    if passphrase.is_empty() {
        return Err(CliError::Usage(format!(
            "{KEY_ENV} is set but empty — supply a non-empty passphrase"
        )));
    }
    StoreKey::from_passphrase(&passphrase).map_err(CliError::from)
}
