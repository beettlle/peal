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
}
