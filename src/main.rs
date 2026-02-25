use std::process::ExitCode;

use clap::Parser;

use peal::cli::{Cli, Commands};
use peal::config::PealConfig;
use peal::error::PealError;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("peal: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Run(args) => {
            let config_path = args.config.clone();
            let config =
                PealConfig::load(config_path.as_deref(), &args)?;

            if !config.plan_path.exists() {
                return Err(PealError::PlanFileNotFound {
                    path: config.plan_path.clone(),
                }
                .into());
            }
            if !config.plan_path.is_file() {
                return Err(PealError::InvalidPlanFile {
                    path: config.plan_path.clone(),
                }
                .into());
            }
            if !config.repo_path.exists() {
                return Err(PealError::RepoPathNotFound {
                    path: config.repo_path.clone(),
                }
                .into());
            }
            if !config.repo_path.is_dir() {
                return Err(PealError::RepoNotDirectory {
                    path: config.repo_path.clone(),
                }
                .into());
            }

            eprintln!(
                "peal: plan={} repo={} agent_cmd={} model={} parallel={} timeout={}s",
                config.plan_path.display(),
                config.repo_path.display(),
                config.agent_cmd,
                config.model.as_deref().unwrap_or("auto"),
                config.parallel,
                config.phase_timeout_sec,
            );
            eprintln!("peal: config loaded, ready for execution (no behavior yet)");

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn run_fails_when_plan_file_missing() {
        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            "/nonexistent/plan.md",
            "--repo",
            "/tmp",
        ])
        .unwrap();

        let result = run(cli);
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("does not exist"),
            "expected 'does not exist', got: {err_msg}"
        );
    }

    #[test]
    fn run_fails_when_repo_not_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo something").unwrap();

        let fake_repo = dir.path().join("not-a-dir.txt");
        fs::write(&fake_repo, "not a directory").unwrap();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            plan_path.to_str().unwrap(),
            "--repo",
            fake_repo.to_str().unwrap(),
        ])
        .unwrap();

        let result = run(cli);
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("not a directory"),
            "expected 'not a directory', got: {err_msg}"
        );
    }

    #[test]
    fn run_succeeds_with_valid_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo something").unwrap();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            plan_path.to_str().unwrap(),
            "--repo",
            dir.path().to_str().unwrap(),
        ])
        .unwrap();

        run(cli).expect("should succeed with valid plan file and repo directory");
    }

    #[test]
    fn run_succeeds_with_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo something").unwrap();

        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            format!(
                "plan_path = {:?}\nrepo_path = {:?}\n",
                plan_path.to_str().unwrap(),
                dir.path().to_str().unwrap(),
            ),
        )
        .unwrap();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--config",
            cfg_path.to_str().unwrap(),
        ])
        .unwrap();

        run(cli).expect("should succeed when plan and repo come from config file");
    }

    #[test]
    fn run_fails_without_plan_or_repo() {
        let cli = Cli::try_parse_from(["peal", "run"]).unwrap();

        let result = run(cli);
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("plan_path is required"),
            "expected 'plan_path is required', got: {err_msg}"
        );
    }
}
