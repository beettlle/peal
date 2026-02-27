use std::path::PathBuf;

const CURSOR_CLI_INSTALL_URL: &str = "https://docs.cursor.com/cli";

/// User-facing message for invalid/missing plan file (PRD §4 and edge cases table).
#[derive(Debug, thiserror::Error)]
pub enum PealError {
    #[error("Invalid or missing plan file.")]
    InvalidPlanFile { path: PathBuf },

    #[error("Target path is not a directory: {path}")]
    RepoNotDirectory { path: PathBuf },

    #[error("Invalid or missing plan file.")]
    PlanFileNotFound { path: PathBuf },

    #[error("Repo path does not exist: {path}")]
    RepoPathNotFound { path: PathBuf },

    #[error("Target path is not a git repository: {path}")]
    RepoNotGitRepo { path: PathBuf },

    #[error(
        "Cursor CLI command '{cmd}' not found on PATH. \
         Install it from {CURSOR_CLI_INSTALL_URL}"
    )]
    AgentCmdNotFound { cmd: String },

    #[error("Phase {phase} failed to start: {detail}")]
    PhaseSpawnFailed { phase: u32, detail: String },

    #[error("Phase {phase} timed out after {timeout_sec}s")]
    PhaseTimedOut { phase: u32, timeout_sec: u64 },

    #[error("Phase {phase} exited with code {exit_code:?}")]
    PhaseNonZeroExit {
        phase: u32,
        exit_code: Option<i32>,
        stderr: String,
    },

    #[error("Failed to parse environment variable '{var}': {detail}")]
    ConfigEnvParseError { var: String, detail: String },

    #[error("Task {index} not found in plan (available: {available:?})")]
    TaskNotFound { index: u32, available: Vec<u32> },

    #[error("Failed to read state file {path}: {detail}")]
    StateReadFailed { path: PathBuf, detail: String },

    #[error("Failed to write state file {path}: {detail}")]
    StateWriteFailed { path: PathBuf, detail: String },

    #[error("stet start failed: {detail}")]
    StetStartFailed { detail: String },

    #[error("stet run failed: {detail}")]
    StetRunFailed { detail: String },

    #[error("stet finish failed: {detail}")]
    StetFinishFailed { detail: String },

    #[error(
        "Task {task_index}: {remaining_count} stet findings remain after {rounds} address round(s)\ncommit: {commit_hash}\nstet review:\n{stet_review}"
    )]
    StetFindingsRemain {
        task_index: u32,
        rounds: u32,
        remaining_count: usize,
        commit_hash: String,
        stet_review: String,
    },

    #[error("Invalid on_findings_remaining value '{value}' (expected \"fail\" or \"warn\")")]
    InvalidOnFindingsRemaining { value: String },

    #[error("Invalid stet_dismiss_patterns reason '{value}' (expected one of: false_positive, already_correct, wrong_suggestion, out_of_scope)")]
    InvalidStetDismissReason { value: String },

    #[error("Invalid on_stet_fail value '{value}' (expected \"fail\", \"retry_once\", or \"skip\")")]
    InvalidOnStetFail { value: String },

    #[error("Plan normalization failed: {detail}")]
    NormalizationFailed { detail: String },

    #[error("Normalization prompt file {path}: {detail}")]
    NormalizePromptFileFailed { path: PathBuf, detail: String },

    /// Normalized plan output could not be parsed (no canonical tasks found).
    /// Includes a bounded snippet of the normalized output for debugging.
    #[error("Normalized plan output could not be parsed (no canonical tasks found). Snippet:\n{snippet}")]
    NormalizationParseFailed { snippet: String },

    #[error("Phase 1 returned empty or invalid plan (task {task_index}): {detail}")]
    Phase1PlanTextInvalid { task_index: u32, detail: String },

    /// Run stopped because the number of consecutive task failures reached the configured cap.
    /// Automation can detect this condition by error type/message or exit code 3.
    #[error("Run stopped: {count} consecutive task failure(s) reached cap {cap}.")]
    ConsecutiveTaskFailuresCapReached { count: u32, cap: u32 },

    #[error("Commit after Phase 2 failed: {detail}")]
    CommitAfterPhase2Failed { detail: String },
}
