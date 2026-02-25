use std::path::PathBuf;

/// Configuration for a PEAL run.
///
/// In SP-0.2, this will support loading from TOML file, env vars, and CLI args
/// with precedence: CLI > env > file. For now it holds only what the CLI provides.
#[derive(Debug)]
pub struct PealConfig {
    pub plan_path: PathBuf,
    pub repo_path: PathBuf,
}
