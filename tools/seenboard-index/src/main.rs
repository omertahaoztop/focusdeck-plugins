use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use seenboard_index::{Entry, Target, check_tag, set_entry};

#[derive(Parser)]
#[command(about = "Updates the seenboard plugin index")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Writes or replaces one plugin entry in the index
    Set {
        #[arg(long)]
        index: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        version: String,
        /// <triple>=<url>,<sha256>
        #[arg(long = "target", required = true)]
        targets: Vec<String>,
    },
    /// Checks that the tag matches <name>-v<version>
    CheckTag {
        #[arg(long)]
        tag: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: String,
    },
}

fn parse_target(spec: &str) -> Result<(String, Target), String> {
    let bad = || format!("Target must look like <triple>=<url>,<sha256>: {spec}");
    let (triple, rest) = spec.split_once('=').ok_or_else(bad)?;
    let (url, sha256) = rest.rsplit_once(',').ok_or_else(bad)?;
    Ok((
        triple.to_string(),
        Target {
            url: url.to_string(),
            sha256: sha256.to_string(),
        },
    ))
}

fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Set {
            index,
            name,
            description,
            version,
            targets,
        } => {
            let mut parsed = BTreeMap::new();
            for spec in &targets {
                let (triple, target) = parse_target(spec)?;
                if parsed.insert(triple.clone(), target).is_some() {
                    return Err(format!("Target given twice: {triple}"));
                }
            }
            let current = std::fs::read_to_string(&index)
                .map_err(|e| format!("Could not read {}: {e}", index.display()))?;
            let entry = Entry {
                name,
                description,
                version,
                targets: parsed,
            };
            let updated = set_entry(&current, entry)?;
            std::fs::write(&index, updated)
                .map_err(|e| format!("Could not write {}: {e}", index.display()))
        }
        Command::CheckTag { tag, name, version } => check_tag(&tag, &name, &version),
    }
}

fn main() -> ExitCode {
    match run(Cli::parse().command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_specs_split_at_the_first_equals_and_the_last_comma() {
        let cases = [
            (
                "x86_64-unknown-linux-gnu=https://example.com/a,abc",
                ("x86_64-unknown-linux-gnu", "https://example.com/a", "abc"),
            ),
            (
                "arm=https://example.com/a,b?x=1,abc",
                ("arm", "https://example.com/a,b?x=1", "abc"),
            ),
            ("arm=,", ("arm", "", "")),
        ];
        for (spec, (triple, url, sha256)) in cases {
            let (name, target) = parse_target(spec).unwrap();
            assert_eq!(name, triple, "{spec}");
            assert_eq!(target.url, url, "{spec}");
            assert_eq!(target.sha256, sha256, "{spec}");
        }
    }

    #[test]
    fn target_specs_without_both_separators_are_refused() {
        for spec in [
            "",
            "arm",
            "arm=https://example.com/a",
            "https://example.com/a,abc",
        ] {
            assert_eq!(
                parse_target(spec).unwrap_err(),
                format!("Target must look like <triple>=<url>,<sha256>: {spec}"),
            );
        }
    }
}
