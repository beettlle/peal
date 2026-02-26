use std::process::ExitCode;
use std::time::Duration;

#[cfg(all(unix, test))]
use std::os::unix::fs::PermissionsExt;

use clap::Parser;
use tracing::{error, info, warn};

use peal::cli::{Cli, Commands};
use peal::config::PealConfig;
use peal::cursor;
use peal::plan;
use peal::plan_prompt;
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
        Commands::Prompt(args) => {
            let prompt = plan_prompt::plan_instructions_prompt();
            match &args.output {
                Some(path) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(path, prompt)?;
                }
                None => println!("{prompt}"),
            }
            Ok(())
        }
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

            let mut run_stet_path = stet_path.clone();
            if let Some(ref sp) = stet_path {
                info!("starting stet session");
                let start_result = match config.on_stet_fail.as_str() {
                    "retry_once" => {
                        match stet::start_session(
                            sp,
                            config.stet_start_ref.as_deref(),
                            &config.stet_start_extra_args,
                            &config.repo_path,
                            Some(Duration::from_secs(config.phase_timeout_sec)),
                        ) {
                            Ok(out) => Ok(out),
                            Err(e) => {
                                warn!(err = %e, "stet start failed, retrying once");
                                stet::start_session(
                                    sp,
                                    config.stet_start_ref.as_deref(),
                                    &config.stet_start_extra_args,
                                    &config.repo_path,
                                    Some(Duration::from_secs(config.phase_timeout_sec)),
                                )
                            }
                        }
                    }
                    "skip" => stet::start_session(
                        sp,
                        config.stet_start_ref.as_deref(),
                        &config.stet_start_extra_args,
                        &config.repo_path,
                        Some(Duration::from_secs(config.phase_timeout_sec)),
                    )
                    .or_else(|e| {
                        warn!(err = %e, "stet start failed; stet phase skipped for this run");
                        run_stet_path = None;
                        Ok(stet::StetOutput {
                            stdout: String::new(),
                            stderr: String::new(),
                        })
                    }),
                    _ => stet::start_session(
                        sp,
                        config.stet_start_ref.as_deref(),
                        &config.stet_start_extra_args,
                        &config.repo_path,
                        Some(Duration::from_secs(config.phase_timeout_sec)),
                    ),
                };
                let stet_out = start_result?;
                if run_stet_path.is_some() {
                    info!(
                        stdout_len = stet_out.stdout.len(),
                        stderr_len = stet_out.stderr.len(),
                        "stet session started"
                    );
                }
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

            let plan_content = std::fs::read_to_string(&config.plan_path).map_err(|e| {
                let peal_err = if e.kind() == std::io::ErrorKind::NotFound {
                    peal::error::PealError::PlanFileNotFound {
                        path: config.plan_path.clone(),
                    }
                } else {
                    peal::error::PealError::InvalidPlanFile {
                        path: config.plan_path.clone(),
                    }
                };
                anyhow::anyhow!(peal_err)
            })?;

            let normalize_enabled = config.normalize_plan || args.normalize;

            let parsed = if plan::is_canonical_plan_format(&plan_content) {
                plan::parse_plan(&plan_content)?
            } else if normalize_enabled {
                info!("plan format not canonical, normalizing via agent");
                let mut parsed_plan = None;
                let attempts = 1 + config.normalize_retry_count;
                for attempt in 0..attempts {
                    let normalized = plan::normalize_via_agent(&plan_content, &agent_path, &config)
                        .map_err(anyhow::Error::from)?;
                    match plan::parse_plan_or_fail_with_snippet(&normalized) {
                        Ok(p) => {
                            parsed_plan = Some(p);
                            break;
                        }
                        Err(peal::error::PealError::NormalizationParseFailed { snippet }) => {
                            if attempt + 1 < attempts {
                                warn!(
                                    attempt = attempt + 1,
                                    retries_left = attempts - attempt - 1,
                                    "normalized output did not parse, retrying normalization"
                                );
                            } else {
                                return Err(peal::error::PealError::NormalizationParseFailed {
                                    snippet,
                                }
                                .into());
                            }
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                parsed_plan.expect("normalize loop exits with Some(parsed) or return Err")
            } else {
                plan::parse_plan(&plan_content)?
            };

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

            if parsed.tasks.is_empty() {
                return Err(peal::error::PealError::InvalidPlanFile {
                    path: config.plan_path.clone(),
                }
                .into());
            }

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

            let run_result = runner::run_scheduled(
                &agent_path,
                &config,
                &parsed,
                &mut peal_state,
                &config.state_dir,
                run_stet_path.as_deref(),
            );

            if let Some(ref sp) = run_stet_path {
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

            if !config.post_run_commands.is_empty() {
                let timeout = config
                    .post_run_timeout_sec
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| Duration::from_secs(config.phase_timeout_sec));
                info!(
                    count = config.post_run_commands.len(),
                    "running post-run command(s)"
                );
                for cmd in &config.post_run_commands {
                    let cmd = cmd.trim();
                    if cmd.is_empty() {
                        continue;
                    }
                    info!(command = %cmd, "running post-run command");
                    match peal::subprocess::run_command_string(cmd, &config.repo_path, Some(timeout))
                    {
                        None => {}
                        Some(Ok(result)) => {
                            const TRUNCATE_BYTES: usize = 2048;
                            let truncate = |s: &str, max: usize| -> String {
                                if s.len() <= max {
                                    s.to_string()
                                } else {
                                    let end = s.floor_char_boundary(max);
                                    s[..end].to_string()
                                }
                            };
                            if result.success() {
                                info!(
                                    stdout_len = result.stdout.len(),
                                    stderr_len = result.stderr.len(),
                                    exit_code = ?result.exit_code,
                                    "post-run command succeeded"
                                );
                                if !result.stdout.is_empty() {
                                    info!(stdout = %truncate(result.stdout.trim(), TRUNCATE_BYTES), "post-run stdout");
                                }
                                if !result.stderr.is_empty() {
                                    info!(stderr = %truncate(result.stderr.trim(), TRUNCATE_BYTES), "post-run stderr");
                                }
                            } else {
                                warn!(
                                    stdout_len = result.stdout.len(),
                                    stderr_len = result.stderr.len(),
                                    exit_code = ?result.exit_code,
                                    timed_out = result.timed_out,
                                    "post-run command failed (best-effort)"
                                );
                                if !result.stdout.is_empty() {
                                    warn!(stdout = %truncate(result.stdout.trim(), TRUNCATE_BYTES), "post-run stdout");
                                }
                                if !result.stderr.is_empty() {
                                    warn!(stderr = %truncate(result.stderr.trim(), TRUNCATE_BYTES), "post-run stderr");
                                }
                            }
                        }
                        Some(Err(e)) => {
                            warn!(error = %e, "post-run command spawn failed (best-effort)");
                        }
                    }
                }
            }

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
            err_msg.contains("Invalid or missing plan file"),
            "expected 'Invalid or missing plan file', got: {err_msg}"
        );
    }

    #[test]
    fn run_fails_when_plan_has_no_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "No ## Task headings here.\n").unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .ok();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            plan_path.to_str().unwrap(),
            "--repo",
            dir.path().to_str().unwrap(),
            "--agent-cmd",
            "echo",
            "--stet-path",
            "/nonexistent",
        ])
        .unwrap();

        let result = run(cli);
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Invalid or missing plan file"),
            "expected 'Invalid or missing plan file', got: {err_msg}"
        );
    }

    #[test]
    fn run_fails_when_agent_cmd_not_found() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo it.").unwrap();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            plan_path.to_str().unwrap(),
            "--repo",
            dir.path().to_str().unwrap(),
            "--agent-cmd",
            "nonexistent-binary-xyz",
            "--stet-path",
            "/nonexistent",
        ])
        .unwrap();

        let result = run(cli);
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("not found"), "expected 'not found' in error: {}", msg);
        assert!(msg.contains("docs.cursor.com/cli"), "expected install link in error: {}", msg);
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
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
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
            "--stet-path",
            "/nonexistent",
        ])
        .unwrap();

        run(cli).expect("should succeed with valid plan file and repo directory");
    }

    /// Canonical plan with --normalize: no normalization call; parse directly and run.
    #[test]
    fn run_canonical_plan_with_normalize_flag_parses_without_agent_normalization() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
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
            "--normalize",
            "--stet-path",
            "/nonexistent",
        ])
        .unwrap();

        run(cli).expect("canonical plan with --normalize should parse directly and succeed");
    }

    /// Non-canonical plan with --normalize: agent is invoked once; stub echoes canonical plan.
    #[test]
    #[cfg(unix)]
    fn run_non_canonical_with_normalize_invokes_agent_then_parses() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "Some PRD or notes. No ## Task headings.").unwrap();

        let stub = dir.path().join("stub_agent.sh");
        fs::write(&stub, "#!/bin/sh\nprintf '%s\\n' '## Task 1' 'Do it.'\n").unwrap();
        let mut perms = fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub, perms).unwrap();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            plan_path.to_str().unwrap(),
            "--repo",
            dir.path().to_str().unwrap(),
            "--agent-cmd",
            stub.to_str().unwrap(),
            "--normalize",
            "--stet-path",
            "/nonexistent",
        ])
        .unwrap();

        run(cli).expect("non-canonical + --normalize should invoke stub, then parse and run");
    }

    /// Non-canonical + --normalize with stub that returns non-canonical output; retry 0 -> NormalizationParseFailed with snippet.
    #[test]
    #[cfg(unix)]
    fn run_normalize_parse_failure_returns_error_with_snippet() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "Some PRD. No ## Task headings.").unwrap();

        let stub = dir.path().join("stub_echo_garbage.sh");
        fs::write(
            &stub,
            "#!/bin/sh\necho 'Agent returned non-canonical output.'\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub, perms).unwrap();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            plan_path.to_str().unwrap(),
            "--repo",
            dir.path().to_str().unwrap(),
            "--agent-cmd",
            stub.to_str().unwrap(),
            "--normalize",
            "--normalize-retry-count",
            "0",
            "--stet-path",
            "/nonexistent",
        ])
        .unwrap();

        let result = run(cli);
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("could not be parsed"),
            "expected parse-failure error, got: {err_msg}"
        );
        assert!(
            err_msg.contains("Agent returned") || err_msg.contains("Snippet:"),
            "error should include snippet of output, got: {err_msg}"
        );
    }

    /// Stub returns bad output on first call, canonical on second; normalize_retry_count=1 -> success after two normalization attempts. Agent is invoked 4 times total: 2 for normalize (retry once) and 2 for phase 1 + phase 2.
    #[test]
    #[cfg(unix)]
    fn run_normalize_retry_succeeds_on_second_attempt() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "PRD. No ## Task headings.").unwrap();

        let stub = dir.path().join("stub_retry.sh");
        fs::write(
            &stub,
            r#"#!/bin/sh
# Agent runs with cwd = repo; use a counter file in cwd.
f=norm_count
c=0
[ -f "$f" ] && c=$(cat "$f")
c=$((c+1))
echo "$c" > "$f"
if [ "$c" -eq 1 ]; then
  echo "garbage"
  exit 0
fi
printf '%s\n' '## Task 1' 'Do it.'
"#,
        )
        .unwrap();
        let mut perms = fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stub, perms).unwrap();

        let cli = Cli::try_parse_from([
            "peal",
            "run",
            "--plan",
            plan_path.to_str().unwrap(),
            "--repo",
            dir.path().to_str().unwrap(),
            "--agent-cmd",
            stub.to_str().unwrap(),
            "--normalize",
            "--normalize-retry-count",
            "1",
            "--stet-path",
            "/nonexistent",
        ])
        .unwrap();

        run(cli).expect("retry should succeed on second normalization");

        let count: u32 = fs::read_to_string(dir.path().join("norm_count"))
            .unwrap_or_default()
            .trim()
            .parse()
            .unwrap_or(0);
        // Two normalization attempts (first fails parse, second succeeds) plus phase 1 + phase 2 = 4 agent invocations.
        assert_eq!(count, 4, "expected 4 agent invocations (2 normalize + phase1 + phase2), got {count}");
    }

    #[test]
    fn run_succeeds_with_config_file() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
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
            Cli::try_parse_from(["peal", "run", "--config", cfg_path.to_str().unwrap(), "--stet-path", "/nonexistent"]).unwrap();

        run(cli).expect("should succeed when plan and repo come from config file");
    }

    #[test]
    fn run_with_task_flag_runs_single_task() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
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
            "--stet-path",
            "/nonexistent",
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
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
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
            "--stet-path",
            "/nonexistent",
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

    #[test]
    fn prompt_writes_template_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("plan-prompt.txt");

        let cli = Cli::try_parse_from([
            "peal",
            "prompt",
            "--output",
            out_path.to_str().expect("temp dir path is valid UTF-8"),
        ])
        .unwrap();

        run(cli).expect("prompt --output should succeed");

        let content = fs::read_to_string(&out_path).expect("output file should exist");
        assert!(
            content.contains("## Task"),
            "template must describe task heading format"
        );
        assert!(
            content.contains("(parallel)"),
            "template must describe parallel marker"
        );
    }

    #[test]
    fn run_with_failing_post_run_command_still_exits_success() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo something").unwrap();

        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            format!(
                "plan_path = {:?}\nrepo_path = {:?}\nagent_cmd = \"echo\"\npost_run_commands = [\"false\"]\n",
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
            "--stet-path",
            "/nonexistent",
        ])
        .unwrap();

        let result = run(cli);
        result.expect("post-run failure is best-effort; peal should still exit success");
    }

    #[test]
    fn run_with_post_run_commands_echo_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo something").unwrap();

        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            format!(
                "plan_path = {:?}\nrepo_path = {:?}\nagent_cmd = \"echo\"\npost_run_commands = [\"echo\", \"hello\"]\n",
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
            "--stet-path",
            "/nonexistent",
        ])
        .unwrap();

        run(cli).expect("run with post_run_commands echo hello should succeed");
    }
}
