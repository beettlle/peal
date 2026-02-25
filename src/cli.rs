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
    Run(RunArgs),
}

/// Arguments for the `run` subcommand.
///
/// `--plan` and `--repo` can also be set via config file or env vars
/// (`PEAL_PLAN_PATH`, `PEAL_REPO_PATH`). Precedence: CLI > env > file.
#[derive(Debug, Clone, clap::Args)]
pub struct RunArgs {
    /// Path to the markdown plan file.
    #[arg(long)]
    pub plan: Option<PathBuf>,

    /// Path to the target repository root.
    #[arg(long)]
    pub repo: Option<PathBuf>,

    /// Path to a TOML configuration file.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Cursor CLI binary name or path (default: "agent").
    #[arg(long)]
    pub agent_cmd: Option<String>,

    /// Model override (omit for Auto).
    #[arg(long)]
    pub model: Option<String>,

    /// Sandbox mode (default: "disabled").
    #[arg(long)]
    pub sandbox: Option<String>,

    /// Directory for state persistence (default: ".peal").
    #[arg(long)]
    pub state_dir: Option<PathBuf>,

    /// Per-phase timeout in seconds (default: 300).
    #[arg(long)]
    pub phase_timeout_sec: Option<u64>,

    /// Enable parallel execution of parallel-marked tasks.
    #[arg(long, default_value_t = false)]
    pub parallel: bool,

    /// Maximum concurrent Cursor CLI processes (default: 4).
    #[arg(long)]
    pub max_parallel: Option<u32>,

    /// Maximum stet address-findings rounds per task (default: 3).
    #[arg(long)]
    pub max_address_rounds: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn run_subcommand_parses_required_args() {
        let cli = Cli::try_parse_from(["peal", "run", "--plan", "tasks.md", "--repo", "/tmp/repo"])
            .expect("should parse valid args");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.plan, Some(PathBuf::from("tasks.md")));
                assert_eq!(args.repo, Some(PathBuf::from("/tmp/repo")));
            }
        }
    }

    #[test]
    fn run_subcommand_accepts_no_plan_no_repo() {
        let cli = Cli::try_parse_from(["peal", "run"])
            .expect("should parse with no --plan or --repo (they come from config)");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.plan, None);
                assert_eq!(args.repo, None);
            }
        }
    }

    #[test]
    fn run_subcommand_parses_all_optional_flags() {
        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            "p.md",
            "--repo",
            "/r",
            "--config",
            "peal.toml",
            "--agent-cmd",
            "cursor-agent",
            "--model",
            "gpt-5.2",
            "--sandbox",
            "enabled",
            "--state-dir",
            ".my-state",
            "--phase-timeout-sec",
            "600",
            "--parallel",
            "--max-parallel",
            "8",
            "--max-address-rounds",
            "5",
        ])
        .expect("should parse all flags");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.config, Some(PathBuf::from("peal.toml")));
                assert_eq!(args.agent_cmd.as_deref(), Some("cursor-agent"));
                assert_eq!(args.model.as_deref(), Some("gpt-5.2"));
                assert_eq!(args.sandbox.as_deref(), Some("enabled"));
                assert_eq!(args.state_dir, Some(PathBuf::from(".my-state")));
                assert_eq!(args.phase_timeout_sec, Some(600));
                assert!(args.parallel);
                assert_eq!(args.max_parallel, Some(8));
                assert_eq!(args.max_address_rounds, Some(5));
            }
        }
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
