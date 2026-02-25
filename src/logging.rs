use std::path::Path;

use std::sync::Once;

use tracing::Level;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::{SubscriberInitExt, TryInitError};
use tracing_subscriber::{EnvFilter, Layer};

const DEFAULT_LOG_LEVEL: &str = "info";
const ENV_VAR_NAME: &str = "PEAL_LOG";

/// Initialize the global tracing subscriber.
///
/// Output goes to stderr by default. When `log_file` is provided, output
/// also goes to that file (appending). The env filter is resolved with
/// precedence: `PEAL_LOG` env var > `log_level` argument > default (`info`).
///
/// Structured fields (phase, task_index, command, exit_code, duration_ms)
/// are attached via `tracing::Span` and `tracing::event!` at call sites.
static INIT: Once = Once::new();

pub fn init(log_level: Option<&str>, log_file: Option<&Path>) -> anyhow::Result<()> {
    let mut init_err: Option<anyhow::Error> = None;

    INIT.call_once(|| {
        if let Err(e) = try_init(log_level, log_file) {
            init_err = Some(e);
        }
    });

    match init_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn try_init(log_level: Option<&str>, log_file: Option<&Path>) -> anyhow::Result<()> {
    let filter = build_filter(log_level);

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_thread_ids(false)
        .with_ansi(true)
        .compact();

    let result: Result<(), TryInitError> = match log_file {
        Some(path) => {
            let file = open_log_file(path)?;
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(file.with_max_level(Level::TRACE))
                .with_target(false)
                .with_thread_ids(false)
                .with_ansi(false)
                .json();

            tracing_subscriber::registry()
                .with(stderr_layer.with_filter(filter))
                .with(file_layer)
                .try_init()
        }
        None => tracing_subscriber::registry()
            .with(stderr_layer.with_filter(filter))
            .try_init(),
    };

    result.map_err(|e| anyhow::anyhow!("failed to initialize logging: {e}"))
}

fn build_filter(log_level: Option<&str>) -> EnvFilter {
    // PEAL_LOG env var takes highest precedence (handled by EnvFilter::try_from_env).
    EnvFilter::try_from_env(ENV_VAR_NAME).unwrap_or_else(|_| {
        let directive = log_level.unwrap_or(DEFAULT_LOG_LEVEL);
        EnvFilter::new(directive)
    })
}

fn open_log_file(path: &Path) -> anyhow::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                anyhow::anyhow!(
                    "failed to create log file directory {}: {e}",
                    parent.display()
                )
            })?;
        }
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| anyhow::anyhow!("failed to open log file {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_filter_uses_default_when_no_override() {
        let filter = build_filter(None);
        let display = format!("{filter}");
        assert!(
            display.contains("info"),
            "expected 'info' default, got: {display}"
        );
    }

    #[test]
    fn build_filter_uses_explicit_level() {
        let filter = build_filter(Some("debug"));
        let display = format!("{filter}");
        assert!(
            display.contains("debug"),
            "expected 'debug', got: {display}"
        );
    }

    #[test]
    fn build_filter_accepts_directive_syntax() {
        let filter = build_filter(Some("peal=trace,warn"));
        let display = format!("{filter}");
        assert!(
            display.contains("peal=trace"),
            "expected 'peal=trace', got: {display}"
        );
    }

    #[test]
    fn open_log_file_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("nested").join("deep").join("peal.log");

        let file = open_log_file(&log_path);
        assert!(file.is_ok(), "should create parent dirs and open file");
        assert!(log_path.exists());
    }

    #[test]
    fn open_log_file_appends() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("peal.log");

        {
            let mut f = open_log_file(&log_path).unwrap();
            write!(f, "line1\n").unwrap();
        }
        {
            let mut f = open_log_file(&log_path).unwrap();
            write!(f, "line2\n").unwrap();
        }

        let contents = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            contents.contains("line1") && contents.contains("line2"),
            "expected both lines, got: {contents}"
        );
    }
}
