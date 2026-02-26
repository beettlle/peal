use std::path::PathBuf;

const CURSOR_CLI_INSTALL_URL: &str = "https://docs.cursor.com/cli";

#[derive(Debug, thiserror::Error)]
pub enum PealError {
    #[error("Invalid or missing plan file: {path}")]
    InvalidPlanFile { path: PathBuf },

    #[error("Target path is not a directory: {path}")]
    RepoNotDirectory { path: PathBuf },

    #[error("Plan file does not exist: {path}")]
    PlanFileNotFound { path: PathBuf },

    #[error("Repo path does not exist: {path}")]
    RepoPathNotFound { path: PathBuf },

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

    #[error("Task {task_index}: {remaining_count} stet findings remain after {rounds} address round(s)")]
    StetFindingsRemain {
        task_index: u32,
        rounds: u32,
        remaining_count: usize,
    },

    #[error("Invalid on_findings_remaining value '{value}' (expected \"fail\" or \"warn\")")]
    InvalidOnFindingsRemaining { value: String },
}
