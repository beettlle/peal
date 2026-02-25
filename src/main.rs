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
        Commands::Run { plan, repo } => {
            if !plan.exists() {
                return Err(PealError::PlanFileNotFound { path: plan.clone() }.into());
            }
            if !plan.is_file() {
                return Err(PealError::InvalidPlanFile { path: plan.clone() }.into());
            }
            if !repo.exists() {
                return Err(PealError::RepoPathNotFound { path: repo.clone() }.into());
            }
            if !repo.is_dir() {
                return Err(PealError::RepoNotDirectory { path: repo.clone() }.into());
            }

            let _config = PealConfig {
                plan_path: plan.clone(),
                repo_path: repo.clone(),
            };

            eprintln!(
                "peal: plan={} repo={}",
                plan.display(),
                repo.display()
            );
            eprintln!("peal: validated inputs, ready for execution (no behavior yet)");

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
}
