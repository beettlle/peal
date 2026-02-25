use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// PEAL — Plan-Execute-Address Loop.
///
/// Orchestrator that drives the Cursor CLI in three phases per task:
/// create plan, execute plan, run stet and address findings.
#[derive(Debug, Parser)]
#[command(name = "peal", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Run the orchestrator on a plan file against a target repo.
    Run {
        /// Path to the markdown plan file.
        #[arg(long)]
        plan: PathBuf,

        /// Path to the target repository root.
        #[arg(long)]
        repo: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn run_subcommand_parses_valid_args() {
        let cli = Cli::try_parse_from(["peal", "run", "--plan", "tasks.md", "--repo", "/tmp/repo"])
            .expect("should parse valid args");

        match cli.command {
            Commands::Run { plan, repo } => {
                assert_eq!(plan, PathBuf::from("tasks.md"));
                assert_eq!(repo, PathBuf::from("/tmp/repo"));
            }
        }
    }

    #[test]
    fn run_subcommand_rejects_missing_plan() {
        let result = Cli::try_parse_from(["peal", "run", "--repo", "/tmp/repo"]);
        let err = result.expect_err("should fail without --plan");
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn run_subcommand_rejects_missing_repo() {
        let result = Cli::try_parse_from(["peal", "run", "--plan", "tasks.md"]);
        let err = result.expect_err("should fail without --repo");
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn no_subcommand_shows_error() {
        let result = Cli::try_parse_from(["peal"]);
        let err = result.expect_err("should fail without subcommand");
        assert_eq!(err.kind(), ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand);
    }

    #[test]
    fn unknown_subcommand_rejected() {
        let result = Cli::try_parse_from(["peal", "unknown"]);
        let err = result.expect_err("should reject unknown subcommand");
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }
}
