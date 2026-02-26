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

    /// Print a prompt template for an LLM to produce a PEAL-compatible plan.
    Prompt(PromptArgs),
}

/// Arguments for the `prompt` subcommand.
#[derive(Debug, Clone, clap::Args)]
pub struct PromptArgs {
    /// Write the prompt to this file instead of stdout.
    #[arg(long)]
    pub output: Option<PathBuf>,
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

    /// Per-phase timeout in seconds (default: 1800 = 30 minutes).
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

    /// Behavior when stet findings persist after all address rounds.
    /// "fail" (default) returns an error; "warn" logs a warning and continues.
    #[arg(long)]
    pub on_findings_remaining: Option<String>,

    /// Run only the task with this index.
    #[arg(long, conflicts_with = "from_task")]
    pub task: Option<u32>,

    /// Run from this task index to the end of the plan.
    #[arg(long, conflicts_with = "task")]
    pub from_task: Option<u32>,

    /// Log level filter (default: "info"). Supports tracing directives
    /// (e.g. "debug", "peal=trace,warn"). Overridden by PEAL_LOG env var.
    #[arg(long)]
    pub log_level: Option<String>,

    /// Path to a log file. When set, structured JSON logs are appended here
    /// in addition to the human-readable stderr output.
    #[arg(long)]
    pub log_file: Option<PathBuf>,

    /// Explicit path to the stet binary. When omitted, stet is
    /// auto-detected on PATH; if not found, Phase 3 is skipped.
    #[arg(long)]
    pub stet_path: Option<PathBuf>,

    /// Git ref passed to `stet start <ref>`. When omitted, `stet start`
    /// is invoked without a ref argument.
    #[arg(long)]
    pub stet_start_ref: Option<String>,

    /// Extra arguments for `stet start` (e.g. `--allow-dirty`). Space-separated.
    #[arg(long)]
    pub stet_start_args: Option<String>,

    /// Extra arguments for `stet run` (e.g. `--verify --context 256k`). Space-separated.
    /// Peal always passes `--output=json` for run.
    #[arg(long)]
    pub stet_run_args: Option<String>,

    /// Disable LLM triage for stet findings; use rule-based dismiss patterns only.
    #[arg(long)]
    pub stet_disable_llm_triage: Option<bool>,
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
            Commands::Prompt(_) => unreachable!("test uses run subcommand"),
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
            Commands::Prompt(_) => unreachable!("test uses run subcommand"),
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
            "--stet-start-ref",
            "HEAD~1",
            "--stet-start-args=--allow-dirty",
            "--stet-run-args=--verify --context 256k",
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
                assert_eq!(args.stet_start_ref.as_deref(), Some("HEAD~1"));
                assert_eq!(args.stet_start_args.as_deref(), Some("--allow-dirty"));
                assert_eq!(
                    args.stet_run_args.as_deref(),
                    Some("--verify --context 256k")
                );
            }
            Commands::Prompt(_) => unreachable!("test uses run subcommand"),
        }
    }

    #[test]
    fn task_flag_parses() {
        let cli = Cli::try_parse_from([
            "peal", "run", "--plan", "p.md", "--repo", "/r", "--task", "5",
        ])
        .expect("should parse --task");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.task, Some(5));
                assert_eq!(args.from_task, None);
            }
            Commands::Prompt(_) => unreachable!("test uses run subcommand"),
        }
    }

    #[test]
    fn from_task_flag_parses() {
        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            "p.md",
            "--repo",
            "/r",
            "--from-task",
            "3",
        ])
        .expect("should parse --from-task");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.task, None);
                assert_eq!(args.from_task, Some(3));
            }
            Commands::Prompt(_) => unreachable!("test uses run subcommand"),
        }
    }

    #[test]
    fn task_and_from_task_conflict() {
        let result = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            "p.md",
            "--repo",
            "/r",
            "--task",
            "1",
            "--from-task",
            "2",
        ]);
        let err = result.expect_err("--task and --from-task should conflict");
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn no_subcommand_shows_error() {
        let result = Cli::try_parse_from(["peal"]);
        let err = result.expect_err("should fail without subcommand");
        assert_eq!(
            err.kind(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }

    #[test]
    fn prompt_subcommand_parses_without_output() {
        let cli = Cli::try_parse_from(["peal", "prompt"]).expect("should parse");
        match cli.command {
            Commands::Prompt(args) => assert_eq!(args.output, None),
            _ => panic!("expected Prompt subcommand"),
        }
    }

    #[test]
    fn prompt_subcommand_parses_with_output() {
        let cli = Cli::try_parse_from(["peal", "prompt", "--output", "out.txt"])
            .expect("should parse");
        match cli.command {
            Commands::Prompt(args) => {
                assert_eq!(args.output, Some(PathBuf::from("out.txt")));
            }
            _ => panic!("expected Prompt subcommand"),
        }
    }

    #[test]
    fn unknown_subcommand_rejected() {
        let result = Cli::try_parse_from(["peal", "unknown"]);
        let err = result.expect_err("should reject unknown subcommand");
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }
}
