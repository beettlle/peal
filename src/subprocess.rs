//! Subprocess execution helper (exec-style, no shell).
//!
//! Spawns child processes directly via `execvp` semantics — no intermediate
//! shell — and captures stdout/stderr into bounded buffers.

use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Upper bound on bytes read from each of stdout / stderr to prevent
/// unbounded memory use (10 MiB).
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024;

/// Polling interval while waiting for a child process with a timeout.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Captured output from a subprocess invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    /// `None` when the process was killed due to timeout or the OS did not
    /// report an exit code (e.g. signal termination on Unix).
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

impl CommandResult {
    /// Returns `true` when the process exited with code 0 and was not killed
    /// by timeout.
    pub fn success(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out
    }
}

/// Run `program` with `args` in directory `cwd`, optionally killing the child
/// after `timeout`.
///
/// Stdout and stderr are each capped at 10 MiB. The child is spawned directly
/// (no shell); use `program` as the executable name and `args` for its argv.
pub fn run_command<S: AsRef<OsStr>>(
    program: &str,
    args: &[S],
    cwd: &Path,
    timeout: Option<Duration>,
) -> std::io::Result<CommandResult> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Take the pipe handles so we can read them on dedicated threads,
    // avoiding deadlock when both pipes fill their OS buffers.
    // We set Stdio::piped() above, so take() always returns Some.
    let child_stdout = child.stdout.take().expect("stdout was piped");
    let child_stderr = child.stderr.take().expect("stderr was piped");

    let stdout_handle = std::thread::spawn(move || read_bounded(child_stdout));
    let stderr_handle = std::thread::spawn(move || read_bounded(child_stderr));

    let (timed_out, exit_code) = wait_with_timeout(&mut child, timeout)?;

    let stdout = stdout_handle
        .join()
        .map_err(|e| std::io::Error::other(format!("stdout reader thread panicked: {e:?}")))??;
    let stderr = stderr_handle
        .join()
        .map_err(|e| std::io::Error::other(format!("stderr reader thread panicked: {e:?}")))??;

    Ok(CommandResult {
        stdout,
        stderr,
        exit_code,
        timed_out,
    })
}

/// Wait for the child to exit. If `timeout` is `Some`, poll with `try_wait`
/// and kill the child when the deadline is exceeded.
///
/// # Race Condition Note
///
/// There is a theoretical race where the child exits successfully just as we
/// decide to kill it due to timeout. In this case, we might report a timeout
/// even if the process finished. This is acceptable for our use case: if it's
/// that close to the timeout, treating it as a timeout is safe.
fn wait_with_timeout(
    child: &mut Child,
    timeout: Option<Duration>,
) -> std::io::Result<(bool, Option<i32>)> {
    match timeout {
        None => {
            let status = child.wait()?;
            Ok((false, status.code()))
        }
        Some(duration) => {
            let deadline = Instant::now() + duration;
            loop {
                if let Some(status) = child.try_wait()? {
                    return Ok((false, status.code()));
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok((true, None));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

/// Read up to [`MAX_OUTPUT_BYTES`] from `reader`, returning the result as a
/// (possibly lossy) UTF-8 string.
fn read_bounded(reader: impl Read) -> std::io::Result<String> {
    let mut buf = Vec::new();
    reader.take(MAX_OUTPUT_BYTES).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Parse a single command string into program and args (first token = program, rest = args).
/// Exec-style: no shell, so args with spaces are not supported unless the user runs
/// a single program that does its own parsing (e.g. `bash -c '...'`).
///
/// Returns `None` if the string is empty or only whitespace (caller should skip).
/// Returns `Some(Ok(result))` or `Some(Err(e))` after calling `run_command`.
pub fn run_command_string(
    command: &str,
    cwd: &Path,
    timeout: Option<Duration>,
) -> Option<std::io::Result<CommandResult>> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tokens: Vec<&str> = trimmed.split_ascii_whitespace().collect();
    let program = tokens[0];
    let args: Vec<&str> = tokens[1..].to_vec();
    Some(run_command(program, &args, cwd, timeout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn captures_stdout_from_echo() {
        let result = run_command("echo", &["hello", "world"], &tmp_dir(), None).unwrap();

        assert_eq!(result.stdout.trim(), "hello world");
        assert!(result.stderr.is_empty());
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.timed_out);
        assert!(result.success());
    }

    #[test]
    fn captures_nonzero_exit_code() {
        let result = run_command("false", &[] as &[&str], &tmp_dir(), None).unwrap();

        assert_ne!(result.exit_code, Some(0));
        assert!(!result.success());
        assert!(!result.timed_out);
    }

    #[test]
    fn respects_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let result = run_command("pwd", &[] as &[&str], dir.path(), None).unwrap();

        // Resolve symlinks for macOS where /tmp -> /private/tmp.
        let expected = dir.path().canonicalize().unwrap();
        let actual: PathBuf = result.stdout.trim().into();
        let actual = actual.canonicalize().unwrap_or(actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn timeout_kills_long_running_process() {
        let result = run_command(
            "sleep",
            &["60"],
            &tmp_dir(),
            Some(Duration::from_millis(200)),
        )
        .unwrap();

        assert!(result.timed_out);
        assert!(!result.success());
    }

    #[test]
    fn no_timeout_allows_fast_completion() {
        let result =
            run_command("sleep", &["0"], &tmp_dir(), Some(Duration::from_secs(5))).unwrap();

        assert!(!result.timed_out);
        assert!(result.success());
    }

    #[test]
    fn captures_stderr() {
        // sh -c is used only in this test to produce stderr output;
        // the production code path uses direct exec (no shell).
        let result = run_command("sh", &["-c", "echo err >&2"], &tmp_dir(), None).unwrap();

        assert_eq!(result.stderr.trim(), "err");
    }

    #[test]
    fn spawn_failure_returns_io_error() {
        let result = run_command("nonexistent-binary-xyz", &[] as &[&str], &tmp_dir(), None);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn success_helper_reports_correctly() {
        let ok = CommandResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            timed_out: false,
        };
        assert!(ok.success());

        let failed = CommandResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(1),
            timed_out: false,
        };
        assert!(!failed.success());

        let timed_out = CommandResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            timed_out: true,
        };
        assert!(!timed_out.success());
    }

    #[test]
    fn run_command_string_parses_and_runs_echo_hello() {
        let result = run_command_string("echo hello", &tmp_dir(), None).unwrap().unwrap();

        assert_eq!(result.stdout.trim(), "hello");
        assert!(result.stderr.is_empty());
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.timed_out);
    }

    #[test]
    fn run_command_string_empty_returns_none() {
        assert!(run_command_string("", &tmp_dir(), None).is_none());
        assert!(run_command_string("   ", &tmp_dir(), None).is_none());
    }

    #[test]
    fn run_command_string_single_token() {
        let result = run_command_string("true", &tmp_dir(), None).unwrap().unwrap();

        assert_eq!(result.exit_code, Some(0));
    }
}
