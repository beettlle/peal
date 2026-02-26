//! Stet CLI resolution and session management.
//!
//! Locates the `stet` binary for Phase 3 (review & address findings).
//! Unlike `cursor::resolve_agent_cmd`, a missing `stet` is not an error —
//! it simply means Phase 3 will be skipped.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{info, warn};

use crate::config::PealConfig;
use crate::cursor::is_executable;
use crate::error::PealError;
use crate::phase::{self, PhaseOutput};
use crate::subprocess;

const STET_BINARY: &str = "stet";

/// Resolve the `stet` binary to an absolute path.
///
/// - If `config_path` is `Some`, verifies it exists and is executable.
///   Returns `None` (with a warning) when the explicit path is invalid.
/// - If `config_path` is `None`, searches `PATH` for the `"stet"` binary.
///   Returns `None` silently when not found.
pub fn resolve_stet(config_path: Option<&Path>) -> Option<PathBuf> {
    resolve_stet_with(config_path, std::env::var_os("PATH"))
}

/// Testable inner implementation that accepts an explicit `PATH` value.
fn resolve_stet_with(
    config_path: Option<&Path>,
    path_var: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    if let Some(explicit) = config_path {
        if is_executable(explicit) {
            return Some(explicit.to_path_buf());
        }
        warn!(
            path = %explicit.display(),
            "configured stet_path is not a valid executable; phase 3 will be skipped"
        );
        return None;
    }

    if let Some(paths) = path_var {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(STET_BINARY);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

/// Captured output from a successful `stet start` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StetOutput {
    pub stdout: String,
    pub stderr: String,
}

/// Start a stet review session by invoking `stet start [ref]`.
///
/// - `stet_path`: absolute path to the stet binary.
/// - `start_ref`: optional git ref (e.g. `"HEAD~1"`); omitted means bare `stet start`.
/// - `repo_path`: working directory for the subprocess.
/// - `timeout`: optional per-command timeout.
pub fn start_session(
    stet_path: &Path,
    start_ref: Option<&str>,
    repo_path: &Path,
    timeout: Option<Duration>,
) -> Result<StetOutput, PealError> {
    let stet_str = stet_path.to_string_lossy();
    let mut args: Vec<String> = vec!["start".to_owned()];
    if let Some(r) = start_ref {
        args.push(r.to_owned());
    }

    info!(
        stet = %stet_str,
        ?args,
        cwd = %repo_path.display(),
        "invoking stet start"
    );

    let result = subprocess::run_command(&stet_str, &args, repo_path, timeout).map_err(|e| {
        PealError::StetStartFailed {
            detail: format!("spawn failed: {e}"),
        }
    })?;

    if result.timed_out {
        warn!("stet start timed out");
        return Err(PealError::StetStartFailed {
            detail: "timed out".to_owned(),
        });
    }

    if !result.success() {
        warn!(
            exit_code = ?result.exit_code,
            stderr_len = result.stderr.len(),
            "stet start exited with non-zero code"
        );
        return Err(PealError::StetStartFailed {
            detail: format!(
                "exit code {:?}: {}",
                result.exit_code,
                result.stderr.trim()
            ),
        });
    }

    info!(
        stdout_len = result.stdout.len(),
        stderr_len = result.stderr.len(),
        "stet start succeeded"
    );

    Ok(StetOutput {
        stdout: result.stdout,
        stderr: result.stderr,
    })
}

/// Persist stet state and clean up the worktree by invoking `stet finish`.
///
/// - `stet_path`: absolute path to the stet binary.
/// - `repo_path`: working directory for the subprocess.
/// - `timeout`: optional per-command timeout.
pub fn finish_session(
    stet_path: &Path,
    repo_path: &Path,
    timeout: Option<Duration>,
) -> Result<StetOutput, PealError> {
    let stet_str = stet_path.to_string_lossy();
    let args: Vec<String> = vec!["finish".to_owned()];

    info!(
        stet = %stet_str,
        cwd = %repo_path.display(),
        "invoking stet finish"
    );

    let result = subprocess::run_command(&stet_str, &args, repo_path, timeout).map_err(|e| {
        PealError::StetFinishFailed {
            detail: format!("spawn failed: {e}"),
        }
    })?;

    if result.timed_out {
        warn!("stet finish timed out");
        return Err(PealError::StetFinishFailed {
            detail: "timed out".to_owned(),
        });
    }

    if !result.success() {
        warn!(
            exit_code = ?result.exit_code,
            stderr_len = result.stderr.len(),
            "stet finish exited with non-zero code"
        );
        return Err(PealError::StetFinishFailed {
            detail: format!(
                "exit code {:?}: {}",
                result.exit_code,
                result.stderr.trim()
            ),
        });
    }

    info!(
        stdout_len = result.stdout.len(),
        stderr_len = result.stderr.len(),
        "stet finish succeeded"
    );

    Ok(StetOutput {
        stdout: result.stdout,
        stderr: result.stderr,
    })
}

/// Captured output from a `stet run` invocation, including the findings heuristic result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StetRunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub has_findings: bool,
}

/// Run an incremental stet review by invoking `stet run` in `repo_path`.
///
/// Unlike `start_session`, a non-zero exit code is **not** treated as an error —
/// it is the standard linter convention for "findings present." Only spawn failures
/// and timeouts produce `PealError::StetRunFailed`.
pub fn run_review(
    stet_path: &Path,
    repo_path: &Path,
    timeout: Option<Duration>,
) -> Result<StetRunResult, PealError> {
    let stet_str = stet_path.to_string_lossy();
    let args: Vec<String> = vec!["run".to_owned()];

    info!(
        stet = %stet_str,
        cwd = %repo_path.display(),
        "invoking stet run"
    );

    let result = subprocess::run_command(&stet_str, &args, repo_path, timeout).map_err(|e| {
        PealError::StetRunFailed {
            detail: format!("spawn failed: {e}"),
        }
    })?;

    if result.timed_out {
        warn!("stet run timed out");
        return Err(PealError::StetRunFailed {
            detail: "timed out".to_owned(),
        });
    }

    let has_findings = detect_findings(result.exit_code, &result.stdout);

    info!(
        exit_code = ?result.exit_code,
        has_findings,
        stdout_len = result.stdout.len(),
        stderr_len = result.stderr.len(),
        "stet run completed"
    );

    Ok(StetRunResult {
        stdout: result.stdout,
        stderr: result.stderr,
        exit_code: result.exit_code,
        has_findings,
    })
}

/// Determine whether `stet run` output indicates findings, applied in priority order:
///
/// 1. **JSON (machine-readable):** If stdout parses as a JSON object with a
///    `"findings"` key containing a non-empty array, or a `"count"` key > 0 →
///    findings present. A top-level JSON array with items also means findings.
/// 2. **Exit code heuristic:** If JSON parsing fails, non-zero exit → findings present.
/// 3. **Stdout content heuristic:** If exit code is 0 but stdout contains
///    non-whitespace content → findings present.
pub fn detect_findings(exit_code: Option<i32>, stdout: &str) -> bool {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout) {
        return match &value {
            serde_json::Value::Object(map) => {
                if let Some(findings) = map.get("findings") {
                    if let Some(arr) = findings.as_array() {
                        return !arr.is_empty();
                    }
                }
                if let Some(count) = map.get("count") {
                    if let Some(n) = count.as_u64() {
                        return n > 0;
                    }
                }
                false
            }
            serde_json::Value::Array(arr) => !arr.is_empty(),
            _ => false,
        };
    }

    // Non-JSON fallback: non-zero exit code signals findings.
    if exit_code != Some(0) {
        return true;
    }

    // Exit code 0 but non-whitespace stdout → findings present.
    !stdout.trim().is_empty()
}

/// Extract `suggestion` fields from stet JSON output.
///
/// Supports two shapes:
/// - `{"findings": [{"suggestion": "..."}, ...]}` (object with findings array)
/// - `[{"suggestion": "..."}, ...]` (top-level array)
///
/// Returns `None` when the output is not JSON or no finding has a `suggestion` field.
pub fn extract_suggestions(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;

    let items: &Vec<serde_json::Value> = match &value {
        serde_json::Value::Object(map) => map.get("findings")?.as_array()?,
        serde_json::Value::Array(arr) => arr,
        _ => return None,
    };

    let suggestions: Vec<&str> = items
        .iter()
        .filter_map(|item| item.get("suggestion")?.as_str())
        .collect();

    if suggestions.is_empty() {
        None
    } else {
        Some(suggestions.join("\n"))
    }
}

/// Result of the address-and-recheck loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressLoopOutcome {
    pub rounds_used: u32,
    pub findings_resolved: bool,
    pub last_stet_result: StetRunResult,
}

/// Drive the address → re-run → check loop for a single task.
///
/// Bounded by `config.max_address_rounds` (default 3). Returns early
/// when findings are resolved. After exhausting all rounds, behavior is
/// controlled by `config.on_findings_remaining` (`"fail"` or `"warn"`).
pub fn address_loop(
    agent_path: &Path,
    stet_path: &Path,
    config: &PealConfig,
    task_index: u32,
    initial_result: &StetRunResult,
) -> Result<AddressLoopOutcome, PealError> {
    if !initial_result.has_findings {
        return Ok(AddressLoopOutcome {
            rounds_used: 0,
            findings_resolved: true,
            last_stet_result: initial_result.clone(),
        });
    }

    let timeout = Some(Duration::from_secs(config.phase_timeout_sec));
    let mut current_result = initial_result.clone();

    for round in 1..=config.max_address_rounds {
        info!(
            task_index,
            round,
            max_rounds = config.max_address_rounds,
            "address loop: starting round"
        );

        address_findings(agent_path, config, task_index, &current_result)?;

        let new_result = run_review(stet_path, &config.repo_path, timeout)?;

        if !new_result.has_findings {
            info!(task_index, round, "address loop: findings resolved");
            return Ok(AddressLoopOutcome {
                rounds_used: round,
                findings_resolved: true,
                last_stet_result: new_result,
            });
        }

        info!(
            task_index,
            round,
            "address loop: findings still present after round"
        );
        current_result = new_result;
    }

    if config.on_findings_remaining == "warn" {
        warn!(
            task_index,
            rounds = config.max_address_rounds,
            "findings remain after all address rounds; continuing (on_findings_remaining=warn)"
        );
        return Ok(AddressLoopOutcome {
            rounds_used: config.max_address_rounds,
            findings_resolved: false,
            last_stet_result: current_result,
        });
    }

    Err(PealError::StetFindingsRemain {
        task_index,
        rounds: config.max_address_rounds,
        remaining_count: count_findings(&current_result.stdout),
    })
}

/// Best-effort count of findings from stet stdout. Falls back to 1 when
/// the output is not structured JSON with a countable findings array.
fn count_findings(stdout: &str) -> usize {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout) {
        if let Some(arr) = value.get("findings").and_then(|v| v.as_array()) {
            return arr.len();
        }
        if let Some(arr) = value.as_array() {
            return arr.len();
        }
    }
    1
}

/// High-level convenience function for SP-4.5/SP-4.6.
///
/// Extracts suggestions from the stet result, builds the phase 3 prompt
/// (with optional suggestions block), and invokes the agent.
pub fn address_findings(
    agent_path: &Path,
    config: &PealConfig,
    task_index: u32,
    stet_result: &StetRunResult,
) -> Result<PhaseOutput, PealError> {
    let suggestions = extract_suggestions(&stet_result.stdout);
    phase::run_phase3(
        agent_path,
        config,
        task_index,
        &stet_result.stdout,
        suggestions.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn returns_none_when_not_on_path() {
        let result = resolve_stet_with(None, Some(OsString::from("/empty/dir/that/does/not/exist")));
        assert_eq!(result, None);
    }

    #[test]
    fn returns_none_when_path_var_is_none() {
        let result = resolve_stet_with(None, None);
        assert_eq!(result, None);
    }

    #[test]
    fn returns_none_when_path_var_is_empty() {
        let result = resolve_stet_with(None, Some(OsString::new()));
        assert_eq!(result, None);
    }

    #[test]
    fn returns_some_with_valid_explicit_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("stet");

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .mode(0o755)
                .open(&bin)
                .unwrap();
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&bin, "").unwrap();
        }

        let result = resolve_stet_with(Some(&bin), None);
        assert_eq!(result, Some(bin));
    }

    #[test]
    fn returns_none_for_invalid_explicit_path() {
        let result = resolve_stet_with(Some(Path::new("/no/such/stet")), None);
        assert_eq!(result, None);
    }

    #[test]
    fn finds_stet_in_custom_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("stet");

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .mode(0o755)
                .open(&bin)
                .unwrap();
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&bin, "").unwrap();
        }

        let path_var = OsString::from(dir.path().as_os_str());
        let result = resolve_stet_with(None, Some(path_var));
        assert_eq!(result, Some(bin));
    }

    #[cfg(unix)]
    #[test]
    fn skips_non_executable_file_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("stet");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();

        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();

        let path_var = OsString::from(dir.path().as_os_str());
        let result = resolve_stet_with(None, Some(path_var));
        assert_eq!(result, None);
    }

    #[cfg(unix)]
    #[test]
    fn returns_none_for_explicit_non_executable_file() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("stet");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();

        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();

        let result = resolve_stet_with(Some(&bin), None);
        assert_eq!(result, None);
    }

    #[test]
    fn skips_directory_with_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("stet");
        std::fs::create_dir(&sub).unwrap();

        let path_var = OsString::from(dir.path().as_os_str());
        let result = resolve_stet_with(None, Some(path_var));
        assert_eq!(result, None);
    }

    // -- start_session tests --

    #[test]
    fn start_session_argv_without_ref() {
        let dir = tempfile::tempdir().unwrap();
        let echo = PathBuf::from("/bin/echo");
        let echo = if echo.exists() {
            echo
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let out = start_session(&echo, None, dir.path(), Some(Duration::from_secs(10))).unwrap();

        assert!(
            out.stdout.contains("start"),
            "stdout should contain 'start': {:?}",
            out.stdout
        );
        assert!(
            !out.stdout.contains("HEAD"),
            "stdout should not contain a ref: {:?}",
            out.stdout
        );
    }

    #[test]
    fn start_session_argv_with_ref() {
        let dir = tempfile::tempdir().unwrap();
        let echo = PathBuf::from("/bin/echo");
        let echo = if echo.exists() {
            echo
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let out = start_session(
            &echo,
            Some("HEAD~1"),
            dir.path(),
            Some(Duration::from_secs(10)),
        )
        .unwrap();

        assert!(
            out.stdout.contains("start"),
            "stdout should contain 'start': {:?}",
            out.stdout
        );
        assert!(
            out.stdout.contains("HEAD~1"),
            "stdout should contain the ref: {:?}",
            out.stdout
        );
    }

    #[test]
    fn start_session_fails_on_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let false_path =
            crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        let err = start_session(&false_path, None, dir.path(), Some(Duration::from_secs(10)))
            .unwrap_err();

        match err {
            PealError::StetStartFailed { detail } => {
                assert!(
                    detail.contains("exit code"),
                    "detail should mention exit code: {detail}"
                );
            }
            other => panic!("expected StetStartFailed, got: {other:?}"),
        }
    }

    #[test]
    fn start_session_fails_on_missing_binary() {
        let dir = tempfile::tempdir().unwrap();
        let bad_path = PathBuf::from("/no/such/stet-binary");

        let err = start_session(&bad_path, None, dir.path(), Some(Duration::from_secs(10)))
            .unwrap_err();

        match err {
            PealError::StetStartFailed { detail } => {
                assert!(
                    detail.contains("spawn failed"),
                    "detail should mention spawn failure: {detail}"
                );
            }
            other => panic!("expected StetStartFailed, got: {other:?}"),
        }
    }

    #[test]
    fn start_session_uses_repo_as_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let pwd_path =
            crate::cursor::resolve_agent_cmd("pwd").expect("pwd must exist");

        let out = start_session(&pwd_path, None, dir.path(), Some(Duration::from_secs(10)))
            .unwrap();

        let expected = dir.path().canonicalize().unwrap();
        let actual: PathBuf = out.stdout.trim().into();
        let actual = actual.canonicalize().unwrap_or(actual);
        assert_eq!(actual, expected);
    }

    // -- detect_findings unit tests --

    #[test]
    fn detect_findings_json_object_with_nonempty_findings_array() {
        let stdout = r#"{"findings": [{"id": "abc", "message": "unused var"}]}"#;
        assert!(detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_object_with_empty_findings_array() {
        let stdout = r#"{"findings": []}"#;
        assert!(!detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_object_with_count_nonzero() {
        let stdout = r#"{"count": 3}"#;
        assert!(detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_object_with_count_zero() {
        let stdout = r#"{"count": 0}"#;
        assert!(!detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_top_level_array_with_items() {
        let stdout = r#"[{"id": "f1"}, {"id": "f2"}]"#;
        assert!(detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_top_level_empty_array() {
        let stdout = "[]";
        assert!(!detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_nonjson_nonzero_exit() {
        assert!(detect_findings(Some(1), "some warning text"));
    }

    #[test]
    fn detect_findings_nonjson_exit_zero_nonempty_stdout() {
        assert!(detect_findings(Some(0), "Found 2 issues\n"));
    }

    #[test]
    fn detect_findings_nonjson_exit_zero_empty_stdout() {
        assert!(!detect_findings(Some(0), ""));
    }

    #[test]
    fn detect_findings_nonjson_exit_zero_whitespace_only_stdout() {
        assert!(!detect_findings(Some(0), "   \n\t  \n"));
    }

    #[test]
    fn detect_findings_exit_code_none_nonjson() {
        // Killed by signal (exit_code = None) → non-zero heuristic → findings.
        assert!(detect_findings(None, ""));
    }

    #[test]
    fn detect_findings_json_object_with_extra_fields() {
        let stdout = r#"{"findings": [{"id": "x"}], "version": "1.0", "metadata": {}}"#;
        assert!(detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_object_no_findings_no_count() {
        let stdout = r#"{"status": "ok"}"#;
        assert!(!detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_findings_takes_priority_over_count() {
        // "findings" is checked first; even if "count" is 0, non-empty array wins.
        let stdout = r#"{"findings": [{"id": "a"}], "count": 0}"#;
        assert!(detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_scalar_value() {
        // A bare JSON scalar (e.g. `true`) is valid JSON but not object/array → false.
        assert!(!detect_findings(Some(0), "true"));
    }

    // -- run_review integration tests --

    #[test]
    fn run_review_populates_result_with_output() {
        let dir = tempfile::tempdir().unwrap();
        let echo = PathBuf::from("/bin/echo");
        let echo = if echo.exists() {
            echo
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let result = run_review(&echo, dir.path(), Some(Duration::from_secs(10))).unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("run"), "stdout should echo 'run'");
    }

    #[test]
    fn run_review_nonzero_exit_is_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let false_path =
            crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        let result = run_review(&false_path, dir.path(), Some(Duration::from_secs(10))).unwrap();

        assert_ne!(result.exit_code, Some(0));
        assert!(result.has_findings);
    }

    #[test]
    fn run_review_spawn_failure_returns_stet_run_failed() {
        let dir = tempfile::tempdir().unwrap();
        let bad_path = PathBuf::from("/no/such/stet-binary");

        let err = run_review(&bad_path, dir.path(), Some(Duration::from_secs(10))).unwrap_err();

        match err {
            PealError::StetRunFailed { detail } => {
                assert!(
                    detail.contains("spawn failed"),
                    "detail should mention spawn failure: {detail}"
                );
            }
            other => panic!("expected StetRunFailed, got: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_review_timeout_returns_stet_run_failed() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("slow-stet");
        std::fs::write(&script, "#!/bin/sh\nsleep 60\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let err = run_review(&script, dir.path(), Some(Duration::from_millis(200)))
            .unwrap_err();

        match err {
            PealError::StetRunFailed { detail } => {
                assert!(
                    detail.contains("timed out"),
                    "detail should mention timeout: {detail}"
                );
            }
            other => panic!("expected StetRunFailed, got: {other:?}"),
        }
    }

    #[test]
    fn run_review_uses_repo_as_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let pwd_path =
            crate::cursor::resolve_agent_cmd("pwd").expect("pwd must exist");

        let result = run_review(&pwd_path, dir.path(), Some(Duration::from_secs(10))).unwrap();

        let expected = dir.path().canonicalize().unwrap();
        let actual: PathBuf = result.stdout.trim().into();
        let actual = actual.canonicalize().unwrap_or(actual);
        assert_eq!(actual, expected);
    }

    // -- extract_suggestions tests --

    #[test]
    fn extract_suggestions_from_json_findings() {
        let stdout = r#"{"findings": [{"id": "a", "message": "bad", "suggestion": "Use X instead"}]}"#;
        let result = extract_suggestions(stdout);
        assert_eq!(result, Some("Use X instead".to_owned()));
    }

    #[test]
    fn extract_suggestions_from_top_level_array() {
        let stdout = r#"[{"id": "a", "suggestion": "fix here"}, {"id": "b", "suggestion": "fix there"}]"#;
        let result = extract_suggestions(stdout);
        assert_eq!(result, Some("fix here\nfix there".to_owned()));
    }

    #[test]
    fn extract_suggestions_none_when_no_suggestion_field() {
        let stdout = r#"{"findings": [{"id": "a", "message": "bad"}]}"#;
        assert_eq!(extract_suggestions(stdout), None);
    }

    #[test]
    fn extract_suggestions_none_for_non_json() {
        assert_eq!(extract_suggestions("plain text output\nwith lines"), None);
    }

    #[test]
    fn extract_suggestions_mixed_findings() {
        let stdout = r#"{"findings": [
            {"id": "a", "message": "m1", "suggestion": "fix A"},
            {"id": "b", "message": "m2"},
            {"id": "c", "message": "m3", "suggestion": "fix C"}
        ]}"#;
        let result = extract_suggestions(stdout).unwrap();
        assert!(result.contains("fix A"));
        assert!(result.contains("fix C"));
        assert!(!result.contains("m2"));
    }

    #[test]
    fn extract_suggestions_none_for_empty_findings_array() {
        let stdout = r#"{"findings": []}"#;
        assert_eq!(extract_suggestions(stdout), None);
    }

    #[test]
    fn extract_suggestions_none_for_empty_top_level_array() {
        assert_eq!(extract_suggestions("[]"), None);
    }

    #[test]
    fn extract_suggestions_none_for_scalar_json() {
        assert_eq!(extract_suggestions("true"), None);
        assert_eq!(extract_suggestions("42"), None);
    }

    // -- address_findings integration test --

    #[test]
    fn address_findings_delegates_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let echo_path = PathBuf::from("/bin/echo");
        let actual_echo = if echo_path.exists() {
            echo_path
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let config = crate::config::PealConfig {
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

        let stet_result = StetRunResult {
            stdout: r#"{"findings": [{"id": "f1", "message": "unused", "suggestion": "remove it"}]}"#.to_owned(),
            stderr: String::new(),
            exit_code: Some(1),
            has_findings: true,
        };

        let output = address_findings(&actual_echo, &config, 1, &stet_result).unwrap();

        assert!(
            output.stdout.contains("---STET---"),
            "should contain stet delimiter: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("---SUGGESTIONS---"),
            "should contain suggestions delimiter: {:?}",
            output.stdout
        );
        assert!(
            output.stdout.contains("remove it"),
            "should contain the suggestion text: {:?}",
            output.stdout
        );
    }

    #[test]
    fn address_findings_no_suggestions_when_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let echo_path = PathBuf::from("/bin/echo");
        let actual_echo = if echo_path.exists() {
            echo_path
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let config = crate::config::PealConfig {
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

        let stet_result = StetRunResult {
            stdout: "warning: unused variable `x`".to_owned(),
            stderr: String::new(),
            exit_code: Some(1),
            has_findings: true,
        };

        let output = address_findings(&actual_echo, &config, 1, &stet_result).unwrap();

        assert!(
            output.stdout.contains("---STET---"),
            "should contain stet delimiter: {:?}",
            output.stdout
        );
        assert!(
            !output.stdout.contains("---SUGGESTIONS---"),
            "should NOT contain suggestions delimiter for plain-text: {:?}",
            output.stdout
        );
    }

    // -- count_findings unit tests --

    #[test]
    fn count_findings_json_object_with_findings_array() {
        let stdout = r#"{"findings": [{"id": "a"}, {"id": "b"}]}"#;
        assert_eq!(count_findings(stdout), 2);
    }

    #[test]
    fn count_findings_json_top_level_array() {
        let stdout = r#"[{"id": "a"}, {"id": "b"}, {"id": "c"}]"#;
        assert_eq!(count_findings(stdout), 3);
    }

    #[test]
    fn count_findings_non_json_returns_one() {
        assert_eq!(count_findings("some plain text"), 1);
    }

    #[test]
    fn count_findings_empty_array_returns_zero() {
        assert_eq!(count_findings("[]"), 0);
    }

    // -- address_loop tests --

    #[test]
    fn address_loop_exits_immediately_when_no_findings() {
        let dir = tempfile::tempdir().unwrap();
        let echo = PathBuf::from("/bin/echo");
        let agent = if echo.exists() {
            echo
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };
        let stet = crate::cursor::resolve_agent_cmd("true").expect("true must exist");

        let config = crate::config::PealConfig {
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

        let initial = StetRunResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            has_findings: false,
        };

        let outcome = address_loop(&agent, &stet, &config, 1, &initial).unwrap();

        assert_eq!(outcome.rounds_used, 0);
        assert!(outcome.findings_resolved);
    }

    #[cfg(unix)]
    #[test]
    fn address_loop_resolves_in_one_round() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();

        // Agent stub: does nothing successfully.
        let agent = crate::cursor::resolve_agent_cmd("true").expect("true must exist");

        // Stet stub: returns exit 0 (no findings) on every call.
        // Since address_findings uses `echo` via agent_cmd, we just need the
        // stet binary to report clean on re-run.
        let stet = crate::cursor::resolve_agent_cmd("true").expect("true must exist");

        let config = crate::config::PealConfig {
            agent_cmd: "true".to_owned(),
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

        let initial = StetRunResult {
            stdout: r#"{"findings": [{"id": "f1"}]}"#.to_owned(),
            stderr: String::new(),
            exit_code: Some(1),
            has_findings: true,
        };

        let outcome = address_loop(&agent, &stet, &config, 1, &initial).unwrap();

        assert_eq!(outcome.rounds_used, 1);
        assert!(outcome.findings_resolved);
    }

    #[cfg(unix)]
    #[test]
    fn address_loop_exhausts_rounds_then_fails() {
        let dir = tempfile::tempdir().unwrap();

        // Agent: succeeds (doesn't matter what it does).
        let agent = crate::cursor::resolve_agent_cmd("true").expect("true must exist");

        // Stet: always returns non-zero → always has findings.
        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        let config = crate::config::PealConfig {
            agent_cmd: "true".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 2,
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

        let initial = StetRunResult {
            stdout: "warning: bad code".to_owned(),
            stderr: String::new(),
            exit_code: Some(1),
            has_findings: true,
        };

        let err = address_loop(&agent, &false_path, &config, 3, &initial).unwrap_err();

        match err {
            PealError::StetFindingsRemain {
                task_index,
                rounds,
                ..
            } => {
                assert_eq!(task_index, 3);
                assert_eq!(rounds, 2);
            }
            other => panic!("expected StetFindingsRemain, got: {other:?}"),
        }
    }

    // -- finish_session tests --

    #[test]
    fn finish_session_argv() {
        let dir = tempfile::tempdir().unwrap();
        let echo = PathBuf::from("/bin/echo");
        let echo = if echo.exists() {
            echo
        } else {
            crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
        };

        let out = finish_session(&echo, dir.path(), Some(Duration::from_secs(10))).unwrap();

        assert!(
            out.stdout.contains("finish"),
            "stdout should contain 'finish': {:?}",
            out.stdout
        );
    }

    #[test]
    fn finish_session_fails_on_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let false_path =
            crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        let err = finish_session(&false_path, dir.path(), Some(Duration::from_secs(10)))
            .unwrap_err();

        match err {
            PealError::StetFinishFailed { detail } => {
                assert!(
                    detail.contains("exit code"),
                    "detail should mention exit code: {detail}"
                );
            }
            other => panic!("expected StetFinishFailed, got: {other:?}"),
        }
    }

    #[test]
    fn finish_session_fails_on_missing_binary() {
        let dir = tempfile::tempdir().unwrap();
        let bad_path = PathBuf::from("/no/such/stet-binary");

        let err = finish_session(&bad_path, dir.path(), Some(Duration::from_secs(10)))
            .unwrap_err();

        match err {
            PealError::StetFinishFailed { detail } => {
                assert!(
                    detail.contains("spawn failed"),
                    "detail should mention spawn failure: {detail}"
                );
            }
            other => panic!("expected StetFinishFailed, got: {other:?}"),
        }
    }

    #[test]
    fn finish_session_uses_repo_as_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let pwd_path =
            crate::cursor::resolve_agent_cmd("pwd").expect("pwd must exist");

        let out = finish_session(&pwd_path, dir.path(), Some(Duration::from_secs(10)))
            .unwrap();

        let expected = dir.path().canonicalize().unwrap();
        let actual: PathBuf = out.stdout.trim().into();
        let actual = actual.canonicalize().unwrap_or(actual);
        assert_eq!(actual, expected);
    }

    #[cfg(unix)]
    #[test]
    fn address_loop_exhausts_rounds_then_warns() {
        let dir = tempfile::tempdir().unwrap();

        let agent = crate::cursor::resolve_agent_cmd("true").expect("true must exist");
        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        let config = crate::config::PealConfig {
            agent_cmd: "true".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 2,
            on_findings_remaining: "warn".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
        };

        let initial = StetRunResult {
            stdout: "warning: bad code".to_owned(),
            stderr: String::new(),
            exit_code: Some(1),
            has_findings: true,
        };

        let outcome = address_loop(&agent, &false_path, &config, 2, &initial).unwrap();

        assert_eq!(outcome.rounds_used, 2);
        assert!(!outcome.findings_resolved);
    }
}
