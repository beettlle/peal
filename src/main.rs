use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use tracing::{error, info, warn};

use peal::cli::{Cli, Commands};
use peal::config::PealConfig;
use peal::cursor;
use peal::plan;
use peal::runner;
use peal::state;
use peal::stet;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Run(args) => {
            let config_path = args.config.clone();
            let config = PealConfig::load(config_path.as_deref(), &args)?;

            peal::logging::init(config.log_level.as_deref(), config.log_file.as_deref())?;

            config.validate()?;

            let agent_path = cursor::resolve_agent_cmd(&config.agent_cmd)?;

            let stet_path = stet::resolve_stet(config.stet_path.as_deref());
            match &stet_path {
                Some(p) => info!(stet_path = %p.display(), "stet found, phase 3 enabled"),
                None => info!("stet not found, phase 3 will be skipped"),
            }

            if let Some(ref sp) = stet_path {
                info!("starting stet session");
                let stet_out = stet::start_session(
                    sp,
                    config.stet_start_ref.as_deref(),
                    &config.repo_path,
                    Some(Duration::from_secs(config.phase_timeout_sec)),
                )?;
                info!(
                    stdout_len = stet_out.stdout.len(),
                    stderr_len = stet_out.stderr.len(),
                    "stet session started"
                );
            }

            info!(
                plan = %config.plan_path.display(),
                repo = %config.repo_path.display(),
                agent_cmd = %config.agent_cmd,
                agent_path = %agent_path.display(),
                model = config.model.as_deref().unwrap_or("auto"),
                parallel = config.parallel,
                timeout_sec = config.phase_timeout_sec,
                "config loaded"
            );

            let parsed = plan::parse_plan_file(&config.plan_path)?;

            let parsed = match (args.task, args.from_task) {
                (Some(idx), None) => {
                    info!(task_index = idx, "filtering plan to single task");
                    parsed.filter_single_task(idx)?
                }
                (None, Some(idx)) => {
                    info!(from_task = idx, "filtering plan from task onward");
                    parsed.filter_from_task(idx)?
                }
                (None, None) => parsed,
                _ => unreachable!("clap prevents both --task and --from-task"),
            };

            info!(
                task_count = parsed.tasks.len(),
                segment_count = parsed.segments.len(),
                "plan parsed"
            );

            let mut peal_state = match state::load_state(&config.state_dir)? {
                Some(s) if s.matches_context(&config.plan_path, &config.repo_path) => {
                    info!(
                        completed = s.completed_task_indices.len(),
                        "resumed from existing state"
                    );
                    s
                }
                Some(_) => {
                    eprintln!(
                        "warning: state file does not match current plan/repo paths; starting fresh"
                    );
                    info!("discarding stale state (plan_path or repo_path mismatch)");
                    state::PealState::new(config.plan_path.clone(), config.repo_path.clone())
                }
                None => state::PealState::new(config.plan_path.clone(), config.repo_path.clone()),
            };

            if !peal_state.completed_task_indices.is_empty() {
                let completed: Vec<u32> = peal_state.completed_task_indices.clone();
                let first_incomplete = parsed
                    .tasks
                    .iter()
                    .find(|t| !peal_state.is_task_completed(t.index))
                    .map(|t| t.index);

                info!(
                    ?completed,
                    ?first_incomplete,
                    "resume: previously completed tasks"
                );
            }

            let run_result = runner::run_all(
                &agent_path,
                &config,
                &parsed,
                &mut peal_state,
                &config.state_dir,
                stet_path.as_deref(),
            );

            if let Some(ref sp) = stet_path {
                let timeout = Some(Duration::from_secs(config.phase_timeout_sec));
                match stet::finish_session(sp, &config.repo_path, timeout) {
                    Ok(out) => info!(
                        stdout_len = out.stdout.len(),
                        stderr_len = out.stderr.len(),
                        "stet finish succeeded"
                    ),
                    Err(e) => warn!(%e, "stet finish failed (best-effort)"),
                }
            }

            let results = run_result?;

            for r in &results {
                info!(
                    task_index = r.task_index,
                    plan_text_len = r.plan_text.len(),
                    phase2_stdout_len = r.phase2_stdout.len(),
                    phase3_rounds = r.phase3_outcome.as_ref().map(|o| o.rounds_used),
                    phase3_resolved = r.phase3_outcome.as_ref().map(|o| o.findings_resolved),
                    "task complete"
                );
            }

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
            "--agent-cmd",
            "echo",
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
                "plan_path = {:?}\nrepo_path = {:?}\nagent_cmd = \"echo\"\n",
                plan_path.to_str().unwrap(),
                dir.path().to_str().unwrap(),
            ),
        )
        .unwrap();

        let cli =
            Cli::try_parse_from(["peal", "run", "--config", cfg_path.to_str().unwrap()]).unwrap();

        run(cli).expect("should succeed when plan and repo come from config file");
    }

    #[test]
    fn run_with_task_flag_runs_single_task() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.md");
        fs::write(
            &plan_path,
            "## Task 1\nFirst\n\n## Task 2\nSecond\n\n## Task 3\nThird\n",
        )
        .unwrap();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            plan_path.to_str().unwrap(),
            "--repo",
            dir.path().to_str().unwrap(),
            "--agent-cmd",
            "echo",
            "--state-dir",
            dir.path().join(".peal").to_str().unwrap(),
            "--task",
            "2",
        ])
        .unwrap();

        run(cli).expect("--task 2 should succeed");

        let loaded = peal::state::load_state(&dir.path().join(".peal"))
            .unwrap()
            .expect("state file should exist");
        assert_eq!(
            loaded.completed_task_indices,
            vec![2],
            "only task 2 should be marked completed"
        );
    }

    #[test]
    fn run_with_from_task_flag_runs_tail() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.md");
        fs::write(
            &plan_path,
            "## Task 1\nFirst\n\n## Task 2\nSecond\n\n## Task 3\nThird\n",
        )
        .unwrap();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            plan_path.to_str().unwrap(),
            "--repo",
            dir.path().to_str().unwrap(),
            "--agent-cmd",
            "echo",
            "--state-dir",
            dir.path().join(".peal").to_str().unwrap(),
            "--from-task",
            "2",
        ])
        .unwrap();

        run(cli).expect("--from-task 2 should succeed");

        let loaded = peal::state::load_state(&dir.path().join(".peal"))
            .unwrap()
            .expect("state file should exist");
        assert_eq!(
            loaded.completed_task_indices,
            vec![2, 3],
            "tasks 2 and 3 should be marked completed"
        );
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
