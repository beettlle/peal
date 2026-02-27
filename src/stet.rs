//! Stet CLI resolution and session management.
//!
//! Phase 3 uses stet for review and address. Peal invokes `stet run` with `--output=json`
//! so findings are machine-readable; it requires a stet binary that supports `--output=json`.
//! If stet is not found on PATH (or via `stet_path`), Phase 3 is skipped.
//!
//! ## Supported stet run JSON output shapes
//!
//! Peal treats the following as machine-readable findings output (for detection and parsing):
//! - `{"findings": [...]}` — canonical stet format; each item has `id`, `file`, `line`, `severity`,
//!   `category`, `confidence`, `message`, optional `suggestion`, etc. (stet `docs/cli-extension-contract.md`,
//!   `cli/internal/findings/finding.go`; emitted by `writeFindingsJSON` in stet `main.go`).
//! - `{"count": N}` with N > 0 — backward compatibility.
//! - Top-level array `[...]` — backward compatibility.
//! - Object with alternate key for findings list (e.g. `"issues"`) — non-empty array treated as findings present.
//!
//! Peal uses `stet run --output=json` without `--stream`, so stdout is a single JSON object (or array).
//! Streaming NDJSON is not used.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::config::{PealConfig, StetDismissPattern, STET_DISMISS_REASONS};
use crate::cursor::is_executable;
use crate::error::PealError;
use crate::phase::{self, PhaseOutput};
use crate::subprocess;

const STET_BINARY: &str = "stet";

/// Keys (in order) used to locate the findings array in a JSON object. Canonical stet key first.
const FINDINGS_ARRAY_KEYS: &[&str] = &["findings", "issues"];

/// Returns a reference to the findings array from parsed stet run JSON.
/// Supports object with "findings" or "issues" key, or top-level array. Single place for format logic.
fn findings_array_from_value(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Object(map) => {
            for key in FINDINGS_ARRAY_KEYS {
                if let Some(arr) = map.get(*key).and_then(|v| v.as_array()) {
                    return Some(arr);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => Some(arr),
        _ => None,
    }
}

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

/// Start a stet review session by invoking `stet start [ref] [extra_args...]`.
///
/// - `stet_path`: absolute path to the stet binary.
/// - `start_ref`: optional git ref (e.g. `"HEAD~1"`); omitted means bare `stet start`.
/// - `extra_args`: optional pass-through args (e.g. `["--allow-dirty"]`).
/// - `repo_path`: working directory for the subprocess.
/// - `timeout`: optional per-command timeout.
pub fn start_session(
    stet_path: &Path,
    start_ref: Option<&str>,
    extra_args: &[String],
    repo_path: &Path,
    timeout: Option<Duration>,
) -> Result<StetOutput, PealError> {
    let stet_str = stet_path.to_string_lossy();
    let mut args: Vec<String> = vec!["start".to_owned()];
    if let Some(r) = start_ref {
        args.push(r.to_owned());
    }
    args.extend(extra_args.iter().cloned());

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

/// Build a `StetRunResult` from raw command output using the same findings heuristic as `run_review`.
/// Used when phase 3 runs a custom command (e.g. `stet_commands` last entry) instead of built-in `stet run`.
pub fn run_result_from_output(stdout: String, stderr: String, exit_code: Option<i32>) -> StetRunResult {
    let has_findings = detect_findings(exit_code, &stdout);
    StetRunResult {
        stdout,
        stderr,
        exit_code,
        has_findings,
    }
}

/// How phase 3 (stet review + address) is driven: built-in stet binary or custom command list.
#[derive(Debug, Clone)]
pub enum StetPhase3Mode {
    /// Use the stet binary at the given path; start/run/finish use built-in sequence.
    BuiltIn(PathBuf),
    /// Use custom commands: session start = all commands run once by main; per-task run = last command only; no built-in finish.
    CustomCommands(Vec<String>),
}

/// Run an incremental stet review by invoking `stet run --output=json [extra_args...]` in `repo_path`.
///
/// Peal always passes `--output=json` so stdout is machine-readable; `extra_args` are appended
/// (e.g. `--verify`, `--context 256k`). Requires stet that supports `--output=json`.
///
/// Unlike `start_session`, a non-zero exit code is **not** treated as an error —
/// it is the standard linter convention for "findings present." Only spawn failures
/// and timeouts produce `PealError::StetRunFailed`.
pub fn run_review(
    stet_path: &Path,
    repo_path: &Path,
    extra_args: &[String],
    timeout: Option<Duration>,
) -> Result<StetRunResult, PealError> {
    let stet_str = stet_path.to_string_lossy();
    let mut args: Vec<String> = vec!["run".to_owned(), "--output=json".to_owned()];
    args.extend(extra_args.iter().cloned());

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

    if !has_findings && !result.stdout.is_empty() {
        let snippet_len = result.stdout.len().min(400);
        let snippet = &result.stdout[..snippet_len];
        debug!(
            stet_stdout_snippet = %snippet,
            "stet run completed with no findings; snippet of stdout for format debugging"
        );
    }

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

/// Run a single command string (e.g. the last entry of `stet_commands`) and return a `StetRunResult`.
/// CWD = `repo_path`; timeout applied. Spawn failure or timeout returns `PealError::StetRunFailed`.
/// Used when phase 3 uses custom commands instead of the built-in stet binary.
pub fn run_review_via_command(
    command: &str,
    repo_path: &Path,
    timeout: Option<Duration>,
) -> Result<StetRunResult, PealError> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(PealError::StetRunFailed {
            detail: "custom run command is empty".to_owned(),
        });
    }
    let result = match subprocess::run_command_string(trimmed, repo_path, timeout) {
        None => {
            return Err(PealError::StetRunFailed {
                detail: "custom run command is empty".to_owned(),
            });
        }
        Some(Err(e)) => {
            return Err(PealError::StetRunFailed {
                detail: format!("spawn failed: {e}"),
            });
        }
        Some(Ok(r)) => r,
    };
    if result.timed_out {
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
        "custom stet run command completed"
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
/// 1. **JSON (machine-readable):** If stdout parses as JSON:
///    - Object with non-empty `"findings"` array → findings present.
///    - Object with `"count"` > 0 → findings present.
///    - Object with other known key (e.g. `"issues"`) containing non-empty array → findings present.
///    - Top-level non-empty array → findings present.
/// 2. **Human summary (stet default):** If stdout is not JSON and the last line
///    indicates zero findings (e.g. "0 finding(s)." or "0 finding(s) at X tokens/sec.") →
///    no findings.
/// 3. **Exit code heuristic:** If JSON parsing fails, non-zero exit → findings present.
/// 4. **Stdout content heuristic:** If exit code is 0 but stdout contains
///    non-whitespace content → findings present.
pub fn detect_findings(exit_code: Option<i32>, stdout: &str) -> bool {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout) {
        if let Some(arr) = findings_array_from_value(&value) {
            return !arr.is_empty();
        }
        if let serde_json::Value::Object(map) = &value {
            if let Some(count) = map.get("count") {
                if let Some(n) = count.as_u64() {
                    return n > 0;
                }
            }
        }
        return false;
    }

    // Human-output fallback: stet's writeFindingsHuman prints "0 finding(s)." or "0 finding(s) at X tokens/sec."
    // when there are no findings. Treat those as no findings so we don't falsely loop.
    let trimmed = stdout.trim();
    if exit_code == Some(0) && !trimmed.is_empty() {
        let last_line = trimmed.lines().last().unwrap_or(trimmed).trim();
        if last_line == "0 finding(s)."
            || last_line == "0 finding(s)"
            || last_line.starts_with("0 finding(s) at ")
        {
            return false;
        }
    }

    // Non-JSON fallback: non-zero exit code signals findings.
    if exit_code != Some(0) {
        return true;
    }

    // Exit code 0 but non-whitespace stdout → findings present.
    !trimmed.is_empty()
}

/// One finding parsed from stet run JSON (for triage and dismiss).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFinding {
    pub id: String,
    pub message: String,
    pub suggestion: Option<String>,
    pub path: Option<String>,
}

/// Parse stet run JSON stdout into a list of findings with id, message, suggestion, path.
/// Uses the same format resolution as [`findings_array_from_value`] (findings, issues, or top-level array).
/// Returns None if not JSON or no findings array.
pub fn parse_findings_from_run_json(stdout: &str) -> Option<Vec<ParsedFinding>> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let items = findings_array_from_value(&value)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let obj = item.as_object()?;
        let id = obj.get("id")?.as_str()?.to_owned();
        let message = obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let suggestion = obj.get("suggestion").and_then(|v| v.as_str()).map(String::from);
        let path = obj
            .get("path")
            .or_else(|| obj.get("file"))
            .and_then(|v| v.as_str())
            .map(String::from);
        out.push(ParsedFinding {
            id,
            message,
            suggestion,
            path,
        });
    }
    Some(out)
}

/// Run `stet dismiss <id> <reason>` in the repo. Best-effort: on failure log and continue.
pub(crate) fn dismiss_finding(
    stet_path: &Path,
    repo_path: &Path,
    id: &str,
    reason: &str,
    timeout: Option<Duration>,
) {
    let stet_str = stet_path.to_string_lossy();
    let args = ["dismiss".to_owned(), id.to_owned(), reason.to_owned()];
    info!(
        stet = %stet_str,
        id,
        reason,
        "invoking stet dismiss"
    );
    match subprocess::run_command(&stet_str, &args, repo_path, timeout) {
        Ok(result) => {
            if !result.success() {
                warn!(
                    id,
                    reason,
                    exit_code = ?result.exit_code,
                    "stet dismiss failed; continuing"
                );
            }
        }
        Err(e) => {
            warn!(id, reason, err = %e, "stet dismiss spawn failed; continuing");
        }
    }
}

/// Result of parsing the triage agent response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TriageResult {
    /// Dismiss all findings with the given reason.
    DismissAll(String),
    /// Only these ids should be addressed; dismiss the rest with default reason.
    DismissRest { to_address: Vec<String> },
    /// Unparseable or ambiguous; dismiss none.
    DismissNone,
}

/// Default reason when the model says "nothing to address" or we dismiss all without a specific reason.
const DEFAULT_DISMISS_REASON: &str = "false_positive";

/// Parse free-form triage response into a list of (id, reason) to dismiss.
/// - "Nothing to address" / "No" / "All false positives" etc. -> dismiss all.
/// - "Fix finding X" / "Only Y needs address" -> dismiss rest (ids not in to_address).
/// - Unparseable -> dismiss none.
pub(crate) fn parse_triage_response(response: &str, findings: &[ParsedFinding]) -> TriageResult {
    let lower = response.trim().to_lowercase();
    if lower.is_empty() {
        return TriageResult::DismissNone;
    }

    // Nothing to address -> dismiss all.
    let nothing_phrases = [
        "nothing to address",
        "no, nothing",
        "no nothing",
        "nothing to fix",
        "all false positive",
        "all false positives",
        "no findings to address",
        "no issues to address",
    ];
    if nothing_phrases
        .iter()
        .any(|p| lower.contains(p))
    {
        return TriageResult::DismissAll(DEFAULT_DISMISS_REASON.to_owned());
    }
    let first_line = lower.lines().next().unwrap_or("").trim();
    if (first_line == "no" || first_line == "no." || first_line == "nope") && lower.len() < 20 {
        return TriageResult::DismissAll(DEFAULT_DISMISS_REASON.to_owned());
    }

    // Look for finding ids mentioned in the response as "to address" / "to fix".
    let mut to_address = Vec::new();
    for f in findings {
        if lower.contains(&f.id.to_lowercase()) {
            let before = lower.find(&f.id.to_lowercase()).unwrap_or(0);
            let snippet = lower.get(before.saturating_sub(30)..(before + f.id.len() + 50).min(lower.len())).unwrap_or("");
            if snippet.contains("fix") || snippet.contains("address") || snippet.contains("need") {
                to_address.push(f.id.clone());
            }
        }
    }

    if !to_address.is_empty() && to_address.len() < findings.len() {
        return TriageResult::DismissRest { to_address };
    }

    if to_address.len() == findings.len() {
        return TriageResult::DismissNone;
    }

    TriageResult::DismissNone
}

/// Apply rule-based patterns to decide which findings to dismiss. Returns (id, reason) for each match.
pub(crate) fn triage_by_patterns(
    findings: &[ParsedFinding],
    patterns: &[StetDismissPattern],
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for f in findings {
        let text = format!(
            "{} {}",
            f.message,
            f.path.as_deref().unwrap_or("")
        );
        for p in patterns {
            if text.contains(&p.pattern) {
                out.push((f.id.clone(), p.reason.clone()));
                break;
            }
        }
    }
    out
}

/// Dismiss non-actionable findings and re-run stet. Returns the new run result.
pub fn dismiss_non_actionable_and_rerun(
    stet_path: &Path,
    agent_path: &Path,
    config: &PealConfig,
    run_stdout: &str,
) -> Result<StetRunResult, PealError> {
    let parsed = parse_findings_from_run_json(run_stdout);
    if parsed.is_none() {
        warn!("stet run output was not valid JSON or had no findings array; skipping structured dismiss");
        return run_review(
            stet_path,
            &config.repo_path,
            &config.stet_run_extra_args,
            Some(Duration::from_secs(config.phase_timeout_sec)),
        );
    }
    let findings = parsed.unwrap();
    if findings.is_empty() {
        return run_review(
            stet_path,
            &config.repo_path,
            &config.stet_run_extra_args,
            Some(Duration::from_secs(config.phase_timeout_sec)),
        );
    }

    let to_dismiss: Vec<(String, String)> = if config.stet_disable_llm_triage {
        triage_by_patterns(&findings, &config.stet_dismiss_patterns)
    } else {
        match phase::run_phase3_triage(agent_path, config, run_stdout) {
            Ok(output) => {
                let triage = parse_triage_response(&output.stdout, &findings);
                match triage {
                    TriageResult::DismissAll(reason) => findings
                        .iter()
                        .map(|f| (f.id.clone(), reason.clone()))
                        .collect(),
                    TriageResult::DismissRest { to_address } => {
                        let to_address_set: std::collections::HashSet<_> =
                            to_address.into_iter().collect();
                        findings
                            .iter()
                            .filter(|f| !to_address_set.contains(&f.id))
                            .map(|f| (f.id.clone(), DEFAULT_DISMISS_REASON.to_owned()))
                            .collect()
                    }
                    TriageResult::DismissNone => Vec::new(),
                }
            }
            Err(_) => Vec::new(),
        }
    };

    let timeout = Some(Duration::from_secs(config.phase_timeout_sec));
    for (id, reason) in &to_dismiss {
        let reason = normalize_dismiss_reason(reason);
        if STET_DISMISS_REASONS.contains(&reason.as_str()) {
            dismiss_finding(stet_path, &config.repo_path, id, &reason, timeout);
        }
    }

    run_review(
        stet_path,
        &config.repo_path,
        &config.stet_run_extra_args,
        timeout,
    )
}

fn normalize_dismiss_reason(s: &str) -> String {
    let t = s.trim().to_lowercase();
    if t == "false_positive" || t == "false positive" {
        return "false_positive".to_owned();
    }
    if t == "already_correct" || t == "already correct" {
        return "already_correct".to_owned();
    }
    if t == "wrong_suggestion" || t == "wrong suggestion" {
        return "wrong_suggestion".to_owned();
    }
    if t == "out_of_scope" || t == "out of scope" {
        return "out_of_scope".to_owned();
    }
    DEFAULT_DISMISS_REASON.to_owned()
}

/// Extract `suggestion` fields from stet JSON output.
///
/// Uses the same format resolution as [`findings_array_from_value`] (findings, issues, or top-level array).
/// Returns `None` when the output is not JSON or no finding has a `suggestion` field.
pub fn extract_suggestions(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let items = findings_array_from_value(&value)?;

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

/// Best-effort resolve current HEAD commit hash in the repo. Returns "unknown" on any failure.
fn resolve_head_commit(repo_path: &Path) -> String {
    let output = match Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return "unknown".to_owned(),
    };
    let s = String::from_utf8_lossy(&output.stdout);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Drive the address → re-run → check loop for a single task.
///
/// Bounded by `config.max_address_rounds` (default 5). Returns early
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

        let after_dismiss = dismiss_non_actionable_and_rerun(
            stet_path,
            agent_path,
            config,
            &current_result.stdout,
        )?;
        current_result = after_dismiss;

        if !current_result.has_findings {
            info!(task_index, round, "address loop: findings resolved after dismiss pass");
            return Ok(AddressLoopOutcome {
                rounds_used: round,
                findings_resolved: true,
                last_stet_result: current_result,
            });
        }

        address_findings(agent_path, config, task_index, &current_result)?;

        let new_result = run_review(
            stet_path,
            &config.repo_path,
            &config.stet_run_extra_args,
            timeout,
        )?;

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
        commit_hash: resolve_head_commit(&config.repo_path),
        stet_review: format!(
            "stdout:\n{}\nstderr:\n{}",
            current_result.stdout, current_result.stderr
        ),
    })
}

/// Address loop for custom commands: no dismiss step; after each round we re-run the last command only.
/// `run_last_command` is typically a closure that runs the last entry of `stet_commands` via `run_review_via_command`.
pub fn address_loop_custom<F>(
    agent_path: &Path,
    config: &PealConfig,
    task_index: u32,
    initial_result: &StetRunResult,
    run_last_command: F,
) -> Result<AddressLoopOutcome, PealError>
where
    F: Fn() -> Result<StetRunResult, PealError>,
{
    if !initial_result.has_findings {
        return Ok(AddressLoopOutcome {
            rounds_used: 0,
            findings_resolved: true,
            last_stet_result: initial_result.clone(),
        });
    }

    let mut current_result = initial_result.clone();

    for round in 1..=config.max_address_rounds {
        info!(
            task_index,
            round,
            max_rounds = config.max_address_rounds,
            "address loop (custom): starting round"
        );

        address_findings(agent_path, config, task_index, &current_result)?;

        let new_result = run_last_command()?;

        if !new_result.has_findings {
            info!(task_index, round, "address loop (custom): findings resolved");
            return Ok(AddressLoopOutcome {
                rounds_used: round,
                findings_resolved: true,
                last_stet_result: new_result,
            });
        }

        info!(
            task_index,
            round,
            "address loop (custom): findings still present after round"
        );
        current_result = new_result;
    }

    if config.on_findings_remaining == "warn" {
        warn!(
            task_index,
            rounds = config.max_address_rounds,
            "findings remain after all address rounds (custom); continuing (on_findings_remaining=warn)"
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
        commit_hash: resolve_head_commit(&config.repo_path),
        stet_review: format!(
            "stdout:\n{}\nstderr:\n{}",
            current_result.stdout, current_result.stderr
        ),
    })
}

/// Best-effort count of findings from stet stdout. Falls back to 1 when
/// the output is not structured JSON with a countable findings array.
/// Uses the same format resolution as [`findings_array_from_value`].
fn count_findings(stdout: &str) -> usize {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout) {
        if let Some(arr) = findings_array_from_value(&value) {
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

    /// Returns path to a script that prints cwd and ignores argv (for cwd tests on Unix).
    #[cfg(unix)]
    fn pwd_script_ignoring_args(dir: &tempfile::TempDir) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.path().join("stet_pwd.sh");
        std::fs::write(&script, "#!/bin/sh\npwd\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

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

        let out = start_session(&echo, None, &[], dir.path(), Some(Duration::from_secs(10))).unwrap();

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
            &[],
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

        let err = start_session(&false_path, None, &[], dir.path(), Some(Duration::from_secs(10)))
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

        let err = start_session(&bad_path, None, &[], dir.path(), Some(Duration::from_secs(10)))
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

    #[cfg(unix)]
    #[test]
    fn start_session_uses_repo_as_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let pwd_script = pwd_script_ignoring_args(&dir);

        let out = start_session(&pwd_script, None, &[], dir.path(), Some(Duration::from_secs(10)))
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

    #[test]
    fn detect_findings_human_zero_findings_with_newline() {
        assert!(!detect_findings(Some(0), "0 finding(s).\n"));
    }

    #[test]
    fn detect_findings_human_zero_findings_no_trailing_dot() {
        assert!(!detect_findings(Some(0), "0 finding(s)"));
    }

    #[test]
    fn detect_findings_human_zero_findings_with_tokens_per_sec() {
        assert!(!detect_findings(Some(0), "0 finding(s) at 12.3 tokens/sec.\n"));
    }

    #[test]
    fn detect_findings_human_one_finding() {
        assert!(detect_findings(Some(0), "1 finding.\n"));
    }

    #[test]
    fn detect_findings_human_multiline_last_line_zero_findings() {
        let stdout = "some header\n0 finding(s).";
        assert!(!detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_stet_no_findings_exact() {
        let stdout = r#"{"findings":[]}"#;
        assert!(!detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_stet_with_findings_exact() {
        let stdout = r#"{"findings":[{"id":"abc123","file":"src/lib.rs","line":10,"severity":"warning","category":"maintainability","confidence":0.9,"message":"Consider adding a comment","suggestion":"Add a doc comment"}]}"#;
        assert!(detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_object_issues_nonempty() {
        let stdout = r#"{"issues":[{"id":"x","message":"y"}]}"#;
        assert!(detect_findings(Some(0), stdout));
    }

    #[test]
    fn detect_findings_json_object_issues_empty_no_findings() {
        let stdout = r#"{"issues":[]}"#;
        assert!(!detect_findings(Some(0), stdout));
    }

    // -- parse_findings_from_run_json tests --

    #[test]
    fn parse_findings_from_run_json_object_with_findings() {
        let stdout = r#"{"findings": [{"id": "a1", "message": "unused var", "suggestion": "remove it"}]}"#;
        let out = parse_findings_from_run_json(stdout).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a1");
        assert_eq!(out[0].message, "unused var");
        assert_eq!(out[0].suggestion.as_deref(), Some("remove it"));
        assert_eq!(out[0].path, None);
    }

    #[test]
    fn parse_findings_from_run_json_top_level_array() {
        let stdout = r#"[{"id": "f1"}, {"id": "f2", "message": "msg2"}]"#;
        let out = parse_findings_from_run_json(stdout).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "f1");
        assert_eq!(out[0].message, "");
        assert_eq!(out[1].id, "f2");
        assert_eq!(out[1].message, "msg2");
    }

    #[test]
    fn parse_findings_from_run_json_empty_array() {
        let stdout = r#"{"findings": []}"#;
        let out = parse_findings_from_run_json(stdout).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn parse_findings_from_run_json_none_for_non_json() {
        assert_eq!(parse_findings_from_run_json("not json"), None);
    }

    #[test]
    fn parse_findings_from_run_json_none_for_malformed() {
        assert_eq!(parse_findings_from_run_json(r#"{"findings": [}"#), None);
    }

    #[test]
    fn parse_findings_from_run_json_includes_path_when_present() {
        let stdout = r#"{"findings": [{"id": "x", "message": "m", "path": "src/lib.rs"}]}"#;
        let out = parse_findings_from_run_json(stdout).unwrap();
        assert_eq!(out[0].path.as_deref(), Some("src/lib.rs"));
    }

    #[test]
    fn parse_findings_from_run_json_stet_shaped_full_finding() {
        let stdout = r#"{"findings":[{"id":"f1","file":"cli/main.go","line":42,"severity":"info","category":"maintainability","confidence":0.9,"message":"Consider adding a comment","suggestion":"Add doc comment"}]}"#;
        let out = parse_findings_from_run_json(stdout).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "f1");
        assert_eq!(out[0].message, "Consider adding a comment");
        assert_eq!(out[0].path.as_deref(), Some("cli/main.go"));
        assert_eq!(out[0].suggestion.as_deref(), Some("Add doc comment"));
    }

    #[test]
    fn parse_findings_from_run_json_issues_array_same_shape() {
        let stdout = r#"{"issues":[{"id":"i1","file":"pkg/foo.go","message":"Unused variable"}]}"#;
        let out = parse_findings_from_run_json(stdout).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "i1");
        assert_eq!(out[0].message, "Unused variable");
        assert_eq!(out[0].path.as_deref(), Some("pkg/foo.go"));
    }

    // -- parse_triage_response tests --

    #[test]
    fn parse_triage_nothing_to_address_dismiss_all() {
        let findings = vec![
            ParsedFinding { id: "f1".into(), message: "m1".into(), suggestion: None, path: None },
            ParsedFinding { id: "f2".into(), message: "m2".into(), suggestion: None, path: None },
        ];
        let r = parse_triage_response("No, nothing to address from this review.", &findings);
        match &r {
            TriageResult::DismissAll(reason) => assert_eq!(reason, "false_positive"),
            _ => panic!("expected DismissAll"),
        }
    }

    #[test]
    fn parse_triage_nothing_to_address_variants() {
        let findings = vec![ParsedFinding { id: "f1".into(), message: "m".into(), suggestion: None, path: None }];
        for response in ["Nothing to address", "All false positives", "No findings to address.", "No."] {
            let r = parse_triage_response(response, &findings);
            assert!(matches!(r, TriageResult::DismissAll(_)), "response {:?}", response);
        }
    }

    #[test]
    fn parse_triage_fix_finding_abc_dismiss_rest() {
        let findings = vec![
            ParsedFinding { id: "abc123".into(), message: "fix this".into(), suggestion: None, path: None },
            ParsedFinding { id: "def456".into(), message: "noise".into(), suggestion: None, path: None },
        ];
        // Avoid "fix" near def456 (e.g. "false") so only abc123 is to_address.
        let r = parse_triage_response("Only finding abc123 needs a fix. The rest are noise.", &findings);
        match &r {
            TriageResult::DismissRest { to_address } => {
                assert_eq!(to_address.len(), 1);
                assert_eq!(to_address[0], "abc123");
            }
            _ => panic!("expected DismissRest"),
        }
    }

    #[test]
    fn parse_triage_unparseable_dismiss_none() {
        let findings = vec![ParsedFinding { id: "f1".into(), message: "m".into(), suggestion: None, path: None }];
        assert!(matches!(parse_triage_response("", &findings), TriageResult::DismissNone));
        assert!(matches!(parse_triage_response("   \n  ", &findings), TriageResult::DismissNone));
        assert!(matches!(parse_triage_response("maybe fix something", &findings), TriageResult::DismissNone));
    }

    // -- triage_by_patterns tests --

    #[test]
    fn triage_by_patterns_empty_patterns_nothing_dismissed() {
        let findings = vec![ParsedFinding { id: "f1".into(), message: "unused".into(), suggestion: None, path: None }];
        let out = triage_by_patterns(&findings, &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn triage_by_patterns_match_message_one_dismissed() {
        let findings = vec![
            ParsedFinding { id: "f1".into(), message: "unused variable".into(), suggestion: None, path: None },
            ParsedFinding { id: "f2".into(), message: "other".into(), suggestion: None, path: None },
        ];
        let patterns = vec![StetDismissPattern { pattern: "unused".to_string(), reason: "false_positive".to_string() }];
        let out = triage_by_patterns(&findings, &patterns);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "f1");
        assert_eq!(out[0].1, "false_positive");
    }

    /// Mock stet that records argv and cwd to capture.txt in repo_path; used to verify dismiss_finding calls.
    #[cfg(unix)]
    fn mock_stet_dismiss_capture_script(dir: &tempfile::TempDir) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.path().join("stet_dismiss.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho \"argv: $@\" >> capture.txt\necho \"cwd: $(pwd)\" >> capture.txt\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    #[cfg(unix)]
    fn dismiss_finding_invokes_stet_with_args_and_repo_cwd() {
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let script = mock_stet_dismiss_capture_script(&dir);
        let repo_path = dir.path().to_path_buf();
        dismiss_finding(
            &script,
            &repo_path,
            "my-id",
            "false_positive",
            Some(Duration::from_secs(5)),
        );
        let capture = std::fs::read_to_string(dir.path().join("capture.txt")).unwrap();
        assert!(capture.contains("dismiss"), "expected dismiss in argv: {}", capture);
        assert!(capture.contains("my-id"), "expected my-id in argv: {}", capture);
        assert!(capture.contains("false_positive"), "expected false_positive in argv: {}", capture);
        assert!(
            capture.contains(repo_path.to_string_lossy().as_ref()),
            "expected repo cwd in capture: {}",
            capture
        );
    }

    /// Stet stub: on "run" prints empty findings JSON; on "dismiss" exits 0.
    #[cfg(unix)]
    fn stet_stub_empty_on_run(dir: &tempfile::TempDir) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.path().join("stet_stub.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
[ "$1" = "run" ] && printf '%s\n' '{"findings":[]}'
exit 0
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    /// Agent stub that prints "Nothing to address from this review." for LLM triage tests.
    #[cfg(unix)]
    fn agent_stub_nothing_to_address(dir: &tempfile::TempDir) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.path().join("agent_triage.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
printf '%s\n' "Nothing to address from this review."
exit 0
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    #[cfg(unix)]
    fn dismiss_non_actionable_and_rerun_llm_nothing_to_address_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let stet_path = stet_stub_empty_on_run(&dir);
        let agent_path = agent_stub_nothing_to_address(&dir);
        let config = crate::config::PealConfig {
            agent_cmd: agent_path.to_string_lossy().to_string(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
        };
        let run_stdout = r#"{"findings":[{"id":"f1","message":"unused"}]}"#;
        let result = dismiss_non_actionable_and_rerun(&stet_path, &agent_path, &config, run_stdout).unwrap();
        assert!(!result.has_findings, "expected no findings after dismiss-all and rerun");
    }

    #[test]
    #[cfg(unix)]
    fn dismiss_non_actionable_and_rerun_rule_pattern_dismisses_then_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let stet_path = stet_stub_empty_on_run(&dir);
        let agent_path = agent_stub_nothing_to_address(&dir); // not used when LLM triage disabled
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
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: true,
            stet_dismiss_patterns: vec![crate::config::StetDismissPattern {
                pattern: "unused".to_string(),
                reason: "false_positive".to_string(),
            }],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
        };
        let run_stdout = r#"{"findings":[{"id":"f1","message":"unused variable"}]}"#;
        let result = dismiss_non_actionable_and_rerun(&stet_path, &agent_path, &config, run_stdout).unwrap();
        assert!(!result.has_findings, "expected no findings after pattern-dismiss and rerun");
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

        let result = run_review(&echo, dir.path(), &[], Some(Duration::from_secs(10))).unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("run"), "stdout should echo 'run'");
    }

    #[test]
    fn run_review_nonzero_exit_is_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let false_path =
            crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        let result = run_review(&false_path, dir.path(), &[], Some(Duration::from_secs(10))).unwrap();

        assert_ne!(result.exit_code, Some(0));
        assert!(result.has_findings);
    }

    #[test]
    fn run_review_spawn_failure_returns_stet_run_failed() {
        let dir = tempfile::tempdir().unwrap();
        let bad_path = PathBuf::from("/no/such/stet-binary");

        let err = run_review(&bad_path, dir.path(), &[], Some(Duration::from_secs(10))).unwrap_err();

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

        let err = run_review(&script, dir.path(), &[], Some(Duration::from_millis(200)))
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

    #[cfg(unix)]
    #[test]
    fn run_review_uses_repo_as_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let pwd_script = pwd_script_ignoring_args(&dir);

        let result = run_review(&pwd_script, dir.path(), &[], Some(Duration::from_secs(10))).unwrap();

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

    #[test]
    fn extract_suggestions_stet_shaped() {
        let stdout = r#"{"findings":[{"id":"f1","file":"a.go","line":1,"severity":"info","category":"maintainability","confidence":0.9,"message":"Add comment","suggestion":"Add doc comment here"}]}"#;
        let result = extract_suggestions(stdout);
        assert_eq!(result, Some("Add doc comment here".to_owned()));
    }

    #[test]
    fn extract_suggestions_from_issues_array() {
        let stdout = r#"{"issues":[{"id":"i1","message":"m","suggestion":"fix via X"}]}"#;
        let result = extract_suggestions(stdout);
        assert_eq!(result, Some("fix via X".to_owned()));
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
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
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
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
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

    #[test]
    fn count_findings_issues_array() {
        let stdout = r#"{"issues":[{"id":"a","message":"m1"},{"id":"b","message":"m2"}]}"#;
        assert_eq!(count_findings(stdout), 2);
    }

    #[test]
    fn count_findings_stet_shaped() {
        let stdout = r#"{"findings":[{"id":"f1","file":"a.go","line":1,"severity":"warning","category":"style","confidence":1.0,"message":"msg"}]}"#;
        assert_eq!(count_findings(stdout), 1);
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
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
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
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
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
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
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

    #[cfg(unix)]
    #[test]
    fn finish_session_uses_repo_as_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let pwd_script = pwd_script_ignoring_args(&dir);

        let out = finish_session(&pwd_script, dir.path(), Some(Duration::from_secs(10)))
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
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
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
