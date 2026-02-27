//! Run summary: build and write run_summary.json on successful run completion.

use std::fs;
use std::io::Write;
use std::path::Path;

use chrono::Utc;
use serde::Serialize;

use crate::config::PealConfig;
use crate::runner::RunOutcome;

/// Summary of a completed run, written when exit code is 0 or 2.
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    /// Task indices that completed without failure and without remaining findings.
    pub tasks_completed: Vec<u32>,
    /// Task indices that were attempted but failed (only non-empty when continue_with_remaining_tasks).
    pub tasks_failed: Vec<u32>,
    /// Task indices that completed phase 2 and phase 3 ran but findings_resolved == false.
    pub tasks_with_remaining_findings: Vec<u32>,
    /// Exit code used for this run (0 or 2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u8>,
    /// Plan path from config (for context).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_path: Option<String>,
    /// Repo path from config (for context).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<String>,
    /// ISO8601 timestamp when the run completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// Build RunSummary from RunOutcome and config. Fills tasks_completed, tasks_failed,
/// tasks_with_remaining_findings from results and failed_task_indices; optional fields from config.
pub fn build_summary(
    outcome: &RunOutcome,
    config: &PealConfig,
    exit_code: u8,
) -> RunSummary {
    let results = &outcome.results;
    let failed = &outcome.failed_task_indices;

    let tasks_completed: Vec<u32> = results
        .iter()
        .filter(|r| {
            !failed.contains(&r.task_index)
                && r.phase3_outcome
                    .as_ref()
                    .map_or(true, |o| o.findings_resolved)
        })
        .map(|r| r.task_index)
        .collect();

    let tasks_with_remaining_findings: Vec<u32> = results
        .iter()
        .filter(|r| {
            r.phase3_outcome
                .as_ref()
                .map_or(false, |o| !o.findings_resolved)
        })
        .map(|r| r.task_index)
        .collect();

    RunSummary {
        tasks_completed,
        tasks_failed: failed.clone(),
        tasks_with_remaining_findings,
        exit_code: Some(exit_code),
        plan_path: Some(config.plan_path.display().to_string()),
        repo_path: Some(config.repo_path.display().to_string()),
        completed_at: Some(Utc::now().to_rfc3339()),
    }
}

/// Resolve the path to write the run summary: config.run_summary_path or state_dir/run_summary.json.
pub fn summary_path(config: &PealConfig) -> std::path::PathBuf {
    config
        .run_summary_path
        .clone()
        .unwrap_or_else(|| config.state_dir.join("run_summary.json"))
}

/// Write summary to the given path. Creates parent dirs if needed; writes atomically (temp then rename).
/// Best-effort: on failure logs a warning and does not change exit code.
pub fn write_run_summary(summary: &RunSummary, path: &Path) {
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(
                path = %path.display(),
                err = %e,
                "failed to create parent directory for run summary"
            );
            return;
        }
    }

    let json = match serde_json::to_string_pretty(summary) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                err = %e,
                "failed to serialize run summary"
            );
            return;
        }
    };

    let tmp_path = path.with_extension("json.tmp");
    if let Err(e) = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        Ok(())
    })() {
        tracing::warn!(
            path = %path.display(),
            err = %e,
            "failed to write run summary (temp file)"
        );
        let _ = fs::remove_file(&tmp_path);
        return;
    }

    if fs::rename(&tmp_path, path).is_err() {
        if let Err(e) = fs::write(path, &json) {
            tracing::warn!(
                path = %path.display(),
                err = %e,
                "failed to write run summary (fallback)"
            );
        }
        let _ = fs::remove_file(&tmp_path);
    }
}
