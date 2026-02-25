//! Cursor CLI resolution.
//!
//! Validates that the configured `agent_cmd` binary is reachable before
//! any subprocess invocation, providing a clear error with install link
//! when the command is missing.

use std::path::{Path, PathBuf};

use crate::error::PealError;

/// Resolve `agent_cmd` to an absolute path on the system.
///
/// - If `cmd` contains a path separator it is treated as an explicit path
///   and checked directly.
/// - Otherwise the function searches each directory in `PATH`.
///
/// Returns the first matching file path, or `PealError::AgentCmdNotFound`
/// with an install link when the binary cannot be located.
pub fn resolve_agent_cmd(cmd: &str) -> Result<PathBuf, PealError> {
    resolve_agent_cmd_with(cmd, std::env::var_os("PATH"))
}

/// Testable inner implementation that accepts an explicit `PATH` value.
fn resolve_agent_cmd_with(
    cmd: &str,
    path_var: Option<std::ffi::OsString>,
) -> Result<PathBuf, PealError> {
    if cmd.contains(std::path::MAIN_SEPARATOR) || cmd.contains('/') {
        let p = PathBuf::from(cmd);
        if is_executable(&p) {
            return Ok(p);
        }
        return Err(PealError::AgentCmdNotFound {
            cmd: cmd.to_owned(),
        });
    }

    if let Some(paths) = path_var {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(cmd);
            if is_executable(&candidate) {
                return Ok(candidate);
            }
        }
    }

    Err(PealError::AgentCmdNotFound {
        cmd: cmd.to_owned(),
    })
}

/// Returns `true` when `path` exists and is a regular file.
///
/// On Unix this additionally checks the executable permission bits via
/// `std::os::unix::fs::PermissionsExt`.
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn resolves_echo_on_real_path() {
        let result = resolve_agent_cmd("echo");
        assert!(result.is_ok(), "echo should exist on PATH: {result:?}");
        assert!(result.unwrap().is_file());
    }

    #[test]
    fn fails_for_nonexistent_command() {
        let result = resolve_agent_cmd("peal-nonexistent-binary-xyz-999");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("not found on PATH"),
            "expected 'not found on PATH', got: {msg}"
        );
        assert!(
            msg.contains("docs.cursor.com/cli"),
            "expected install link, got: {msg}"
        );
    }

    #[test]
    fn fails_for_explicit_path_that_does_not_exist() {
        let result = resolve_agent_cmd("/no/such/binary");
        assert!(result.is_err());
    }

    #[test]
    fn resolves_explicit_absolute_path() {
        let resolved = resolve_agent_cmd("sh").expect("sh should exist");
        let result = resolve_agent_cmd(resolved.to_str().unwrap());
        assert!(
            result.is_ok(),
            "absolute path to sh should resolve: {result:?}"
        );
    }

    #[test]
    fn fails_when_path_var_is_empty() {
        let result = resolve_agent_cmd_with("echo", Some(OsString::new()));
        assert!(result.is_err());
    }

    #[test]
    fn fails_when_path_var_is_none() {
        let result = resolve_agent_cmd_with("echo", None);
        assert!(result.is_err());
    }

    #[test]
    fn finds_binary_in_custom_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("my-agent");

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
        let result = resolve_agent_cmd_with("my-agent", Some(path_var));
        assert!(
            result.is_ok(),
            "should find binary in custom PATH: {result:?}"
        );
        assert_eq!(result.unwrap(), bin);
    }

    #[test]
    fn skips_directory_with_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("my-agent");
        std::fs::create_dir(&sub).unwrap();

        let path_var = OsString::from(dir.path().as_os_str());
        let result = resolve_agent_cmd_with("my-agent", Some(path_var));
        assert!(
            result.is_err(),
            "directory should not be treated as executable"
        );
    }

    #[cfg(unix)]
    #[test]
    fn skips_file_without_execute_permission() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("no-exec");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();

        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();

        let path_var = OsString::from(dir.path().as_os_str());
        let result = resolve_agent_cmd_with("no-exec", Some(path_var));
        assert!(
            result.is_err(),
            "file without execute permission should be skipped"
        );
    }
}
