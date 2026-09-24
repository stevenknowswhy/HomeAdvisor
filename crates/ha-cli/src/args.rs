//! Argument parsing: three subcommands, no dependencies.
//!
//! Hand-rolled on purpose — the surface is two flags wide, and a demo
//! harness should not carry an argument-framework dependency to hold it.
//! Parsing is total and unit-tested: unknown flags, missing values, and
//! duplicates are errors, never silently accepted.

use std::path::PathBuf;

use crate::error::CliError;

/// The usage text, printed verbatim for `help` and for every parse error.
pub const USAGE: &str = "\
Home Advisor CLI — the milestone-1 demo: family data -> privacy gate -> receipt

USAGE:
    ha-cli seed --db <path>                        seed the demo family (banded profile only)
    ha-cli research --db <path> --sidecar <url>    run a purpose-limited research request through the gate
    ha-cli receipt --db <path>                     print the egress log (the family-facing privacy screen)

The SQLCipher key is read from the environment at runtime and never persisted
beside the database:

    HA_STORE_KEY=<key> ha-cli seed --db demo.db

The sidecar URL must be loopback (e.g. http://127.0.0.1:8000): the semantic
leak-scan never leaves this machine. Without a reachable sidecar the gate
blocks — that is the fail-closed design working, not a crash.";

/// A parsed invocation. `Help` prints usage to stdout and exits 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Command(Command),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Seed the demo family into the database at `db`.
    Seed { db: PathBuf },
    /// Run the demo research request through the gate.
    Research { db: PathBuf, sidecar_url: String },
    /// Print the egress log.
    Receipt { db: PathBuf },
}

/// Parse raw arguments (program name already stripped).
pub fn parse(args: &[String]) -> Result<Parsed, CliError> {
    match args.first().map(String::as_str) {
        None => Err(usage("expected a subcommand")),
        Some("help") | Some("--help") | Some("-h") => Ok(Parsed::Help),
        Some("seed") => seed_command(&args[1..]),
        Some("research") => research_command(&args[1..]),
        Some("receipt") => receipt_command(&args[1..]),
        Some(other) => Err(usage(&format!("unknown subcommand {other:?}"))),
    }
}

fn seed_command(args: &[String]) -> Result<Parsed, CliError> {
    let mut flags = Flags::parse(args)?;
    let db = flags.required_path("--db")?;
    flags.ensure_empty(&["--db"])?;
    Ok(Parsed::Command(Command::Seed { db }))
}

fn research_command(args: &[String]) -> Result<Parsed, CliError> {
    let mut flags = Flags::parse(args)?;
    let db = flags.required_path("--db")?;
    let sidecar_url = flags.required_value("--sidecar")?;
    flags.ensure_empty(&["--db", "--sidecar"])?;
    Ok(Parsed::Command(Command::Research { db, sidecar_url }))
}

fn receipt_command(args: &[String]) -> Result<Parsed, CliError> {
    let mut flags = Flags::parse(args)?;
    let db = flags.required_path("--db")?;
    flags.ensure_empty(&["--db"])?;
    Ok(Parsed::Command(Command::Receipt { db }))
}

fn usage(detail: &str) -> CliError {
    CliError::Usage(format!("{detail}\n\n{USAGE}"))
}

/// Parsed `--flag value` / `--flag=value` pairs. Order preserved, values
/// consumed exactly once.
struct Flags {
    values: Vec<(String, String)>,
}

impl Flags {
    fn parse(args: &[String]) -> Result<Self, CliError> {
        let mut values = Vec::new();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) => (name.to_string(), Some(value.to_string())),
                None => (arg.clone(), None),
            };
            if !name.starts_with("--") || name.len() <= 2 {
                return Err(usage(&format!("expected a --flag, got {arg:?}")));
            }
            let value = match inline {
                Some(value) => value,
                None => {
                    index += 1;
                    args.get(index)
                        .ok_or_else(|| usage(&format!("flag {name} needs a value")))?
                        .clone()
                }
            };
            values.push((name, value));
            index += 1;
        }
        Ok(Self { values })
    }

    fn required_value(&mut self, name: &str) -> Result<String, CliError> {
        let position = self
            .values
            .iter()
            .position(|(flag, _)| flag == name)
            .ok_or_else(|| usage(&format!("missing required flag {name}")))?;
        let (_, value) = self.values.remove(position);
        Ok(value)
    }

    fn required_path(&mut self, name: &str) -> Result<PathBuf, CliError> {
        self.required_value(name).map(PathBuf::from)
    }

    fn ensure_empty(&self, expected: &[&str]) -> Result<(), CliError> {
        if let Some((flag, _)) = self.values.first() {
            let expected_list = expected.join(", ");
            return Err(usage(&format!(
                "unexpected flag {flag} (expected one of: {expected_list})"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args<const N: usize>(values: [&str; N]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_each_subcommand() {
        let parsed = parse(&args(["seed", "--db", "demo.db"])).unwrap();
        assert_eq!(
            parsed,
            Parsed::Command(Command::Seed {
                db: PathBuf::from("demo.db")
            })
        );

        let parsed = parse(&args([
            "research",
            "--db=demo.db",
            "--sidecar",
            "http://127.0.0.1:8000",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Parsed::Command(Command::Research {
                db: PathBuf::from("demo.db"),
                sidecar_url: "http://127.0.0.1:8000".to_string(),
            })
        );

        let parsed = parse(&args(["receipt", "--db", "demo.db"])).unwrap();
        assert_eq!(
            parsed,
            Parsed::Command(Command::Receipt {
                db: PathBuf::from("demo.db")
            })
        );
    }

    #[test]
    fn help_forms_are_recognized() {
        for form in ["help", "--help", "-h"] {
            assert_eq!(parse(&args([form])).unwrap(), Parsed::Help);
        }
    }

    #[test]
    fn missing_or_unknown_pieces_are_usage_errors() {
        // No subcommand, unknown subcommand.
        assert!(parse(&args([])).is_err());
        assert!(parse(&args(["fly"])).is_err());
        // Missing required flags and values.
        assert!(parse(&args(["seed"])).is_err());
        assert!(parse(&args(["seed", "--db"])).is_err());
        assert!(parse(&args(["research", "--db", "demo.db"])).is_err());
        // Unexpected flags.
        assert!(parse(&args(["seed", "--db", "demo.db", "--sidecar", "x"])).is_err());
        assert!(parse(&args(["seed", "--db", "demo.db", "--what"])).is_err());
        // Positional junk.
        assert!(parse(&args(["seed", "demo.db"])).is_err());
    }
}
