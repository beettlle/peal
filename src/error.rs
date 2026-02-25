use std::path::PathBuf;

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
}
