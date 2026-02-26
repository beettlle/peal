//! Phase invocation: build argv and call the subprocess helper.
//!
//! Each phase constructs a Cursor CLI command and invokes it via
//! `subprocess::run_command`.  This module owns the argv layout so
//! that changes to CLI flags happen in one place.
//!
//! Prompt strings are built exclusively by the `prompt` module; this
//! module only passes them as the final positional arg in the argv.

use std::path::Path;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::config::PealConfig;
use crate::error::PealError;
use crate::prompt;
use crate::subprocess::{self, CommandResult};

/// Captured output from a successful phase invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseOutput {
    /// Stdout from the agent (the plan text for Phase 1).
    pub stdout: String,
    /// Stderr from the agent (diagnostic / log output).
    pub stderr: String,
}

/// Run Phase 1 (plan creation) for a single task.
///
/// Builds the prompt via `prompt::phase1`, constructs the `agent` argv,
/// invokes the subprocess, and returns the captured stdout as the plan
/// text.  On timeout or non-zero exit, returns an error.
pub fn run_phase1(
    agent_path: &Path,
    config: &PealConfig,
    task_index: u32,
    task_content: &str,
) -> Result<PhaseOutput, PealError> {
    let prompt = prompt::phase1(task_content);
    let args = phase1_argv(config, &prompt);
    let timeout = Duration::from_secs(config.phase_timeout_sec);

    let agent_str = agent_path.to_string_lossy();

    info!(
        phase = 1,
        task_index,
        agent = %agent_str,
        timeout_sec = config.phase_timeout_sec,
        "invoking phase 1"
    );
    debug!(phase = 1, task_index, ?args, "phase 1 argv");

    let result = subprocess::run_command(&agent_str, &args, &config.repo_path, Some(timeout))
        .map_err(|e| PealError::PhaseSpawnFailed {
            phase: 1,
            detail: e.to_string(),
        })?;

    check_result(1, task_index, config.phase_timeout_sec, &result)?;

    Ok(PhaseOutput {
        stdout: result.stdout,
        stderr: result.stderr,
    })
}

/// Build the argv (excluding the program name) for a Phase 1 invocation.
///
/// Layout:
/// ```text
/// --print --plan --workspace <repo> --output-format text [--model <m>] <prompt>
/// ```
fn phase1_argv(config: &PealConfig, prompt: &str) -> Vec<String> {
    let mut args = vec![
        "--print".to_owned(),
        "--plan".to_owned(),
        "--workspace".to_owned(),
        config.repo_path.to_string_lossy().into_owned(),
        "--output-format".to_owned(),
        "text".to_owned(),
    ];

    if let Some(ref model) = config.model {
        args.push("--model".to_owned());
        args.push(model.clone());
    }

    args.push(prompt.to_owned());
    args
}

/// Run Phase 2 (plan execution) for a single task.
///
/// Builds the prompt via `prompt::phase2`, constructs the `agent` argv
/// (no `--plan` flag, includes `--sandbox`), invokes the subprocess, and
/// returns the captured output.  On timeout or non-zero exit, returns an error.
pub fn run_phase2(
    agent_path: &Path,
    config: &PealConfig,
    task_index: u32,
    plan_text: &str,
) -> Result<PhaseOutput, PealError> {
    let prompt = prompt::phase2(plan_text);
    let args = phase2_argv(config, &prompt);
    let timeout = Duration::from_secs(config.phase_timeout_sec);

    let agent_str = agent_path.to_string_lossy();

    info!(
        phase = 2,
        task_index,
        agent = %agent_str,
        timeout_sec = config.phase_timeout_sec,
        "invoking phase 2"
    );
    debug!(phase = 2, task_index, ?args, "phase 2 argv");

    let result = subprocess::run_command(&agent_str, &args, &config.repo_path, Some(timeout))
        .map_err(|e| PealError::PhaseSpawnFailed {
            phase: 2,
            detail: e.to_string(),
        })?;

    check_result(2, task_index, config.phase_timeout_sec, &result)?;

    Ok(PhaseOutput {
        stdout: result.stdout,
        stderr: result.stderr,
    })
}

/// Build the argv (excluding the program name) for a Phase 2 invocation.
///
/// Layout:
/// ```text
/// --print --workspace <repo> --sandbox <sandbox> [--model <m>] <prompt>
/// ```
///
/// Phase 2 omits `--plan` (execution, not planning) and includes
/// `--sandbox` from config.
fn phase2_argv(config: &PealConfig, prompt: &str) -> Vec<String> {
    let mut args = vec![
        "--print".to_owned(),
        "--workspace".to_owned(),
        config.repo_path.to_string_lossy().into_owned(),
        "--sandbox".to_owned(),
        config.sandbox.clone(),
    ];

    if let Some(ref model) = config.model {
        args.push("--model".to_owned());
        args.push(model.clone());
    }

    args.push(prompt.to_owned());
    args
}

/// Run Phase 3 (address stet findings) for a single task.
///
/// Builds the prompt via `prompt::phase3_with_suggestions`, constructs the
/// `agent` argv (same layout as Phase 2: no `--plan`, with `--sandbox`),
/// invokes the subprocess, and returns the captured output.  On timeout or
/// non-zero exit, returns an error.
pub fn run_phase3(
    agent_path: &Path,
    config: &PealConfig,
    task_index: u32,
    stet_output: &str,
    suggestions: Option<&str>,
) -> Result<PhaseOutput, PealError> {
    let prompt = prompt::phase3_with_suggestions(stet_output, suggestions);
    let args = phase3_argv(config, &prompt);
    let timeout = Duration::from_secs(config.phase_timeout_sec);

    let agent_str = agent_path.to_string_lossy();

    info!(
        phase = 3,
        task_index,
        agent = %agent_str,
        timeout_sec = config.phase_timeout_sec,
        "invoking phase 3"
    );
    debug!(phase = 3, task_index, ?args, "phase 3 argv");

    let result = subprocess::run_command(&agent_str, &args, &config.repo_path, Some(timeout))
        .map_err(|e| PealError::PhaseSpawnFailed {
            phase: 3,
            detail: e.to_string(),
        })?;

    check_result(3, task_index, config.phase_timeout_sec, &result)?;

    Ok(PhaseOutput {
        stdout: result.stdout,
        stderr: result.stderr,
    })
}

/// Build the argv for a Phase 3 invocation (same layout as Phase 2).
fn phase3_argv(config: &PealConfig, prompt: &str) -> Vec<String> {
    let mut args = vec![
        "--print".to_owned(),
        "--workspace".to_owned(),
        config.repo_path.to_string_lossy().into_owned(),
        "--sandbox".to_owned(),
        config.sandbox.clone(),
    ];

    if let Some(ref model) = config.model {
        args.push("--model".to_owned());
        args.push(model.clone());
    }

    args.push(prompt.to_owned());
    args
}

/// Validate a `CommandResult`, returning an error on timeout or non-zero exit.
fn check_result(
    phase: u32,
    task_index: u32,
    timeout_sec: u64,
    result: &CommandResult,
) -> Result<(), PealError> {
    if result.timed_out {
        warn!(phase, task_index, timeout_sec, "phase timed out");
        return Err(PealError::PhaseTimedOut { phase, timeout_sec });
    }

    if !result.success() {
        warn!(
            phase,
            task_index,
            exit_code = ?result.exit_code,
            stderr_len = result.stderr.len(),
            "phase exited with non-zero code"
        );
        return Err(PealError::PhaseNonZeroExit {
            phase,
            exit_code: result.exit_code,
            stderr: result.stderr.clone(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Helper: build a minimal `PealConfig` for testing argv construction.
    fn test_config(model: Option<&str>) -> PealConfig {
        PealConfig {
            agent_cmd: "agent".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: PathBuf::from("/my/repo"),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: model.map(|s| s.to_owned()),
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 1800,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        }
    }

    // -- argv construction tests --

    #[test]
    fn argv_without_model() {
        let config = test_config(None);
        let args = phase1_argv(&config, "Do the thing.");

        assert_eq!(
            args,
            vec![
                "--print",
                "--plan",
                "--workspace",
                "/my/repo",
                "--output-format",
                "text",
                "Do the thing.",
            ]
        );
    }

    #[test]
    fn argv_with_model() {
        let config = test_config(Some("claude-4-opus"));
        let args = phase1_argv(&config, "Do the thing.");

        assert_eq!(
            args,
            vec![
                "--print",
                "--plan",
                "--workspace",
                "/my/repo",
                "--output-format",
                "text",
                "--model",
                "claude-4-opus",
                "Do the thing.",
            ]
        );
    }

    #[test]
    fn argv_prompt_is_last_arg() {
        let config = test_config(Some("gpt-5"));
        let prompt_text = "Create a plan for implementing this task: do X";
        let args = phase1_argv(&config, prompt_text);

        assert_eq!(
            args.last().unwrap(),
            prompt_text,
            "prompt must be the final positional arg"
        );
    }

    // -- check_result tests --

    #[test]
    fn check_result_success() {
        let result = CommandResult {
            stdout: "plan text".to_owned(),
            stderr: String::new(),
            exit_code: Some(0),
            timed_out: false,
        };
        assert!(check_result(1, 1, 300, &result).is_ok());
    }

    #[test]
    fn check_result_timeout() {
        let result = CommandResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            timed_out: true,
        };
        let err = check_result(1, 1, 300, &result).unwrap_err();
        match err {
            PealError::PhaseTimedOut { phase, timeout_sec } => {
                assert_eq!(phase, 1);
                assert_eq!(timeout_sec, 300);
            }
            other => panic!("expected PhaseTimedOut, got: {other:?}"),
        }
    }

    #[test]
    fn check_result_nonzero_exit() {
        let result = CommandResult {
            stdout: String::new(),
            stderr: "something went wrong".to_owned(),
            exit_code: Some(1),
            timed_out: false,
        };
        let err = check_result(1, 1, 300, &result).unwrap_err();
        match err {
            PealError::PhaseNonZeroExit {
                phase,
                exit_code,
                stderr,
            } => {
                assert_eq!(phase, 1);
                assert_eq!(exit_code, Some(1));
                assert_eq!(stderr, "something went wrong");
            }
            other => panic!("expected PhaseNonZeroExit, got: {other:?}"),
        }
    }

    // -- Phase 2 argv construction tests --

    #[test]
    fn phase2_argv_without_model() {
        let config = test_config(None);
        let args = phase2_argv(&config, "Execute this plan.");

        assert_eq!(
            args,
            vec![
                "--print",
                "--workspace",
                "/my/repo",
                "--sandbox",
                "disabled",
                "Execute this plan.",
            ]
        );
    }

    #[test]
    fn phase2_argv_with_model() {
        let config = test_config(Some("claude-4-opus"));
        let args = phase2_argv(&config, "Execute this plan.");

        assert_eq!(
            args,
            vec![
                "--print",
                "--workspace",
                "/my/repo",
                "--sandbox",
                "disabled",
                "--model",
                "claude-4-opus",
                "Execute this plan.",
            ]
        );
    }

    #[test]
    fn phase2_argv_prompt_is_last_arg() {
        let config = test_config(Some("gpt-5"));
        let prompt_text = "Execute the following plan. Do not re-plan; only implement and test.";
        let args = phase2_argv(&config, prompt_text);

        assert_eq!(
            args.last().unwrap(),
            prompt_text,
            "prompt must be the final positional arg"
        );
    }

    #[test]
    fn phase2_argv_does_not_contain_plan_flag() {
        let config = test_config(Some("model"));
        let args = phase2_argv(&config, "prompt");

        assert!(
            !args.contains(&"--plan".to_owned()),
            "phase 2 must not include --plan flag"
        );
    }

    #[test]
    fn phase2_argv_includes_sandbox() {
        let mut config = test_config(None);
        config.sandbox = "enabled".to_owned();
        let args = phase2_argv(&config, "prompt");

        let sandbox_idx = args.iter().position(|a| a == "--sandbox").unwrap();
        assert_eq!(args[sandbox_idx + 1], "enabled");
    }

    // -- Integration-style tests using real binaries --

    #[test]
    fn run_phase1_with_echo_stub() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "echo".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let echo_path = PathBuf::from("/bin/echo");
        let actual_echo = if echo_path.exists() {
            echo_path
        } else {
            // Fall back to PATH resolution.
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let output = run_phase1(&actual_echo, &config, 1, "Build the widget.").unwrap();

        // echo receives the full argv and prints it to stdout; we verify
        // the prompt appears as the last positional arg.
        assert!(
            output
                .stdout
                .contains("Create a plan for implementing this task:"),
            "stdout should contain the phase 1 instruction: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("---TASK---"),
            "stdout should contain the task delimiter: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("Build the widget."),
            "stdout should contain the task content: {:?}",
            output.stdout
        );
    }

    #[test]
    fn run_phase1_fails_on_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "false".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");
        let err = run_phase1(&false_path, &config, 1, "task").unwrap_err();

        match err {
            PealError::PhaseNonZeroExit { phase, .. } => assert_eq!(phase, 1),
            other => panic!("expected PhaseNonZeroExit, got: {other:?}"),
        }
    }

    #[test]
    fn run_phase1_fails_on_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "sleep".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            // Very short timeout to trigger kill.
            phase_timeout_sec: 1,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let sleep_path = crate::cursor::resolve_agent_cmd("sleep").expect("sleep must exist");
        // sleep receives all the argv (--print --plan ... 60) and tries to
        // sleep for the first numeric-looking arg; but sleep will just fail
        // or run — either way the 1s timeout will fire first.
        let err = run_phase1(&sleep_path, &config, 1, "60").unwrap_err();

        match err {
            PealError::PhaseTimedOut { phase, timeout_sec } => {
                assert_eq!(phase, 1);
                assert_eq!(timeout_sec, 1);
            }
            // sleep may exit non-zero before the timeout because it can't
            // parse the argv — that's acceptable too.
            PealError::PhaseNonZeroExit { phase, .. } => assert_eq!(phase, 1),
            other => panic!("expected PhaseTimedOut or PhaseNonZeroExit, got: {other:?}"),
        }
    }

    #[test]
    fn run_phase1_spawn_failure() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "nonexistent".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let bad_path = PathBuf::from("/no/such/binary");
        let err = run_phase1(&bad_path, &config, 1, "task").unwrap_err();

        match err {
            PealError::PhaseSpawnFailed { phase, detail } => {
                assert_eq!(phase, 1);
                assert!(!detail.is_empty());
            }
            other => panic!("expected PhaseSpawnFailed, got: {other:?}"),
        }
    }

    // -- Phase 2 integration-style tests --

    #[test]
    fn run_phase2_with_echo_stub() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "echo".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let echo_path = PathBuf::from("/bin/echo");
        let actual_echo = if echo_path.exists() {
            echo_path
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let output = run_phase2(&actual_echo, &config, 1, "1. Build widget\n2. Test it").unwrap();

        assert!(
            output.stdout.contains("Execute the following plan"),
            "stdout should contain the phase 2 instruction: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("---PLAN---"),
            "stdout should contain the plan delimiter: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("1. Build widget"),
            "stdout should contain the plan text: {:?}",
            output.stdout
        );
    }

    #[test]
    fn run_phase2_fails_on_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "false".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");
        let err = run_phase2(&false_path, &config, 1, "plan text").unwrap_err();

        match err {
            PealError::PhaseNonZeroExit { phase, .. } => assert_eq!(phase, 2),
            other => panic!("expected PhaseNonZeroExit for phase 2, got: {other:?}"),
        }
    }

    #[test]
    fn run_phase2_spawn_failure() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "nonexistent".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let bad_path = PathBuf::from("/no/such/binary");
        let err = run_phase2(&bad_path, &config, 1, "plan text").unwrap_err();

        match err {
            PealError::PhaseSpawnFailed { phase, detail } => {
                assert_eq!(phase, 2);
                assert!(!detail.is_empty());
            }
            other => panic!("expected PhaseSpawnFailed for phase 2, got: {other:?}"),
        }
    }

    // -- Phase 3 argv construction tests --

    #[test]
    fn phase3_argv_without_model() {
        let config = test_config(None);
        let args = phase3_argv(&config, "Address findings.");

        assert_eq!(
            args,
            vec![
                "--print",
                "--workspace",
                "/my/repo",
                "--sandbox",
                "disabled",
                "Address findings.",
            ]
        );
    }

    #[test]
    fn phase3_argv_with_model() {
        let config = test_config(Some("claude-4-opus"));
        let args = phase3_argv(&config, "Address findings.");

        assert_eq!(
            args,
            vec![
                "--print",
                "--workspace",
                "/my/repo",
                "--sandbox",
                "disabled",
                "--model",
                "claude-4-opus",
                "Address findings.",
            ]
        );
    }

    #[test]
    fn phase3_argv_does_not_contain_plan_flag() {
        let config = test_config(Some("model"));
        let args = phase3_argv(&config, "prompt");

        assert!(
            !args.contains(&"--plan".to_owned()),
            "phase 3 must not include --plan flag"
        );
    }

    #[test]
    fn phase3_argv_prompt_is_last_arg() {
        let config = test_config(Some("gpt-5"));
        let prompt_text = "Address the following stet review findings.";
        let args = phase3_argv(&config, prompt_text);

        assert_eq!(
            args.last().unwrap(),
            prompt_text,
            "prompt must be the final positional arg"
        );
    }

    // -- Phase 3 integration-style tests --

    #[test]
    fn run_phase3_with_echo_stub() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "echo".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let echo_path = PathBuf::from("/bin/echo");
        let actual_echo = if echo_path.exists() {
            echo_path
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let output = run_phase3(&actual_echo, &config, 1, "warning: unused variable `x`", None).unwrap();

        assert!(
            output
                .stdout
                .contains("Address the following stet review findings"),
            "stdout should contain the phase 3 instruction: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("---STET---"),
            "stdout should contain the stet delimiter: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("unused variable"),
            "stdout should contain the stet output: {:?}",
            output.stdout
        );
    }

    #[test]
    fn run_phase3_fails_on_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "false".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");
        let err = run_phase3(&false_path, &config, 1, "stet output", None).unwrap_err();

        match err {
            PealError::PhaseNonZeroExit { phase, .. } => assert_eq!(phase, 3),
            other => panic!("expected PhaseNonZeroExit for phase 3, got: {other:?}"),
        }
    }

    #[test]
    fn run_phase3_spawn_failure() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "nonexistent".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let bad_path = PathBuf::from("/no/such/binary");
        let err = run_phase3(&bad_path, &config, 1, "stet output", None).unwrap_err();

        match err {
            PealError::PhaseSpawnFailed { phase, detail } => {
                assert_eq!(phase, 3);
                assert!(!detail.is_empty());
            }
            other => panic!("expected PhaseSpawnFailed for phase 3, got: {other:?}"),
        }
    }

    #[test]
    fn run_phase3_with_suggestions_echo_stub() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "echo".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let echo_path = PathBuf::from("/bin/echo");
        let actual_echo = if echo_path.exists() {
            echo_path
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let output = run_phase3(
            &actual_echo,
            &config,
            1,
            "finding: bad code",
            Some("Use good code instead"),
        )
        .unwrap();

        assert!(
            output.stdout.contains("---SUGGESTIONS---"),
            "stdout should contain SUGGESTIONS delimiter: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("Use good code instead"),
            "stdout should contain the suggestion text: {:?}",
            output.stdout
        );
    }

    #[test]
    fn run_phase3_without_suggestions_echo_stub() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "echo".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let echo_path = PathBuf::from("/bin/echo");
        let actual_echo = if echo_path.exists() {
            echo_path
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let output = run_phase3(&actual_echo, &config, 1, "finding: bad code", None).unwrap();

        assert!(
            !output.stdout.contains("---SUGGESTIONS---"),
            "stdout should NOT contain SUGGESTIONS delimiter when None: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("---STET---"),
            "stdout should still contain STET delimiter: {:?}",
            output.stdout
        );
    }
}
