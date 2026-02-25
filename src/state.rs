use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::PealError;

/// Persistent state for a peal run, serialized to `.peal/state.json`.
///
/// Tracks which tasks have been completed so that interrupted runs can resume.
/// This struct is pure data + serialization; file I/O lives in a separate module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PealState {
    /// Path to the plan file driving this run.
    pub plan_path: PathBuf,

    /// Path to the target repository.
    pub repo_path: PathBuf,

    /// Sorted, deduplicated indices of successfully completed tasks.
    pub completed_task_indices: Vec<u32>,

    /// Plan text produced by Phase 1, keyed by task index.
    /// `BTreeMap` keeps JSON keys in deterministic order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_plan_by_task: Option<BTreeMap<u32, String>>,

    /// A git ref or timestamp marking the last successful completion point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_completed_ref: Option<String>,
}

impl PealState {
    /// Creates a fresh state with no completed tasks.
    pub fn new(plan_path: PathBuf, repo_path: PathBuf) -> Self {
        Self {
            plan_path,
            repo_path,
            completed_task_indices: Vec::new(),
            last_plan_by_task: None,
            last_completed_ref: None,
        }
    }

    /// Returns `true` if the loaded state matches the current plan and repo paths.
    ///
    /// A mismatch means the state was produced by a different run and should
    /// be discarded rather than used for resume.
    pub fn matches_context(&self, plan_path: &Path, repo_path: &Path) -> bool {
        self.plan_path == plan_path && self.repo_path == repo_path
    }

    /// Returns `true` if the given task index has been marked completed.
    pub fn is_task_completed(&self, index: u32) -> bool {
        self.completed_task_indices.binary_search(&index).is_ok()
    }

    /// Marks a task index as completed, maintaining sorted order and ignoring duplicates.
    pub fn mark_task_completed(&mut self, index: u32) {
        if let Err(pos) = self.completed_task_indices.binary_search(&index) {
            self.completed_task_indices.insert(pos, index);
        }
    }

    /// Returns the canonical state file path within the given state directory.
    pub fn state_file_path(state_dir: &Path) -> PathBuf {
        state_dir.join("state.json")
    }
}

/// Load persisted state from `state_dir/state.json`.
///
/// Returns `Ok(None)` if the file does not exist or contains invalid JSON
/// (the latter also emits a warning to stderr). Returns `Err` on unexpected
/// I/O errors such as permission denied.
pub fn load_state(state_dir: &Path) -> Result<Option<PealState>, PealError> {
    let path = PealState::state_file_path(state_dir);

    let contents = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(PealError::StateReadFailed {
                path,
                detail: e.to_string(),
            });
        }
    };

    match serde_json::from_str::<PealState>(&contents) {
        Ok(state) => Ok(Some(state)),
        Err(e) => {
            eprintln!(
                "warning: ignoring invalid state file {}: {e}",
                path.display()
            );
            tracing::warn!(
                path = %path.display(),
                err = %e,
                "ignoring invalid state file"
            );
            Ok(None)
        }
    }
}

/// Persist state to `state_dir/state.json`.
///
/// Creates `state_dir` if it does not exist. Writes to a temporary file in
/// the same directory and renames for atomicity; falls back to direct write
/// if the rename fails (e.g. cross-device).
pub fn save_state(state: &PealState, state_dir: &Path) -> Result<(), PealError> {
    let path = PealState::state_file_path(state_dir);

    fs::create_dir_all(state_dir).map_err(|e| PealError::StateWriteFailed {
        path: state_dir.to_path_buf(),
        detail: format!("failed to create directory: {e}"),
    })?;

    let json = serde_json::to_string_pretty(state).map_err(|e| PealError::StateWriteFailed {
        path: path.clone(),
        detail: format!("serialization failed: {e}"),
    })?;

    let tmp_path = state_dir.join("state.json.tmp");

    let write_result = (|| -> Result<(), PealError> {
        let mut f = fs::File::create(&tmp_path).map_err(|e| PealError::StateWriteFailed {
            path: tmp_path.clone(),
            detail: e.to_string(),
        })?;
        f.write_all(json.as_bytes())
            .map_err(|e| PealError::StateWriteFailed {
                path: tmp_path.clone(),
                detail: e.to_string(),
            })?;
        f.flush().map_err(|e| PealError::StateWriteFailed {
            path: tmp_path.clone(),
            detail: e.to_string(),
        })?;
        Ok(())
    })();

    write_result?;

    // Atomic rename; fall back to direct write on failure.
    if fs::rename(&tmp_path, &path).is_err() {
        fs::write(&path, &json).map_err(|e| PealError::StateWriteFailed {
            path: path.clone(),
            detail: e.to_string(),
        })?;
        // Best-effort cleanup of temp file after failed rename (e.g. cross-device).
        // Error is ignored; state was already written to path and we are succeeding.
        let _ = fs::remove_file(&tmp_path);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state() -> PealState {
        PealState::new(PathBuf::from("plan.md"), PathBuf::from("/repo"))
    }

    #[test]
    fn new_state_has_empty_defaults() {
        let state = sample_state();
        assert!(state.completed_task_indices.is_empty());
        assert_eq!(state.last_plan_by_task, None);
        assert_eq!(state.last_completed_ref, None);
    }

    #[test]
    fn mark_task_completed_inserts_sorted_and_deduplicates() {
        let mut state = sample_state();
        state.mark_task_completed(3);
        state.mark_task_completed(1);
        state.mark_task_completed(2);
        state.mark_task_completed(1); // duplicate
        assert_eq!(state.completed_task_indices, vec![1, 2, 3]);
    }

    #[test]
    fn matches_context_returns_true_for_matching_paths() {
        let state = PealState::new(PathBuf::from("plan.md"), PathBuf::from("/repo"));
        assert!(state.matches_context(Path::new("plan.md"), Path::new("/repo")));
    }

    #[test]
    fn matches_context_returns_false_on_plan_path_mismatch() {
        let state = PealState::new(PathBuf::from("plan.md"), PathBuf::from("/repo"));
        assert!(!state.matches_context(Path::new("other.md"), Path::new("/repo")));
    }

    #[test]
    fn matches_context_returns_false_on_repo_path_mismatch() {
        let state = PealState::new(PathBuf::from("plan.md"), PathBuf::from("/repo"));
        assert!(!state.matches_context(Path::new("plan.md"), Path::new("/other")));
    }

    #[test]
    fn is_task_completed_reflects_marks() {
        let mut state = sample_state();
        assert!(!state.is_task_completed(1));
        state.mark_task_completed(1);
        assert!(state.is_task_completed(1));
        assert!(!state.is_task_completed(2));
    }

    #[test]
    fn state_file_path_appends_filename() {
        let dir = Path::new("/project/.peal");
        assert_eq!(
            PealState::state_file_path(dir),
            PathBuf::from("/project/.peal/state.json"),
        );
    }

    #[test]
    fn serialization_roundtrip() {
        let mut state = sample_state();
        state.mark_task_completed(0);
        state.mark_task_completed(2);
        state.last_completed_ref = Some("abc123".into());

        let mut plans = BTreeMap::new();
        plans.insert(0, "implement feature X".into());
        state.last_plan_by_task = Some(plans);

        let json = serde_json::to_string_pretty(&state).expect("serialize");
        let restored: PealState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(state, restored);
    }

    #[test]
    fn json_format_matches_expected_structure() {
        let state = sample_state();
        let json: serde_json::Value = serde_json::to_value(&state).expect("to_value");

        assert_eq!(json["plan_path"], "plan.md");
        assert_eq!(json["repo_path"], "/repo");
        assert_eq!(json["completed_task_indices"], serde_json::json!([]));
        // Optional fields with skip_serializing_if should be absent.
        assert!(json.get("last_plan_by_task").is_none());
        assert!(json.get("last_completed_ref").is_none());
    }

    #[test]
    fn json_includes_optional_fields_when_present() {
        let mut state = sample_state();
        state.last_completed_ref = Some("ref-1".into());

        let json: serde_json::Value = serde_json::to_value(&state).expect("to_value");
        assert_eq!(json["last_completed_ref"], "ref-1");
    }

    // -- load_state / save_state tests --

    #[test]
    fn load_state_returns_none_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_state(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_state_returns_none_on_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("state.json");
        fs::write(&state_file, "not valid json {{{").unwrap();

        let result = load_state(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_state_roundtrips_saved_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = sample_state();
        state.mark_task_completed(1);
        state.mark_task_completed(3);

        save_state(&state, dir.path()).unwrap();
        let loaded = load_state(dir.path())
            .unwrap()
            .expect("should load saved state");

        assert_eq!(loaded, state);
    }

    #[test]
    fn save_state_creates_directory_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("c");

        let state = sample_state();
        save_state(&state, &nested).unwrap();

        assert!(nested.join("state.json").exists());
    }

    #[test]
    fn save_state_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();

        let mut state1 = sample_state();
        state1.mark_task_completed(1);
        save_state(&state1, dir.path()).unwrap();

        let mut state2 = sample_state();
        state2.mark_task_completed(1);
        state2.mark_task_completed(2);
        save_state(&state2, dir.path()).unwrap();

        let loaded = load_state(dir.path()).unwrap().expect("should load");
        assert_eq!(loaded.completed_task_indices, vec![1, 2]);
    }

    #[test]
    fn save_state_produces_valid_deserializable_json() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = sample_state();
        state.mark_task_completed(5);
        state.last_completed_ref = Some("deadbeef".into());

        save_state(&state, dir.path()).unwrap();

        let raw = fs::read_to_string(dir.path().join("state.json")).unwrap();
        let parsed: PealState = serde_json::from_str(&raw).expect("valid JSON");
        assert_eq!(parsed, state);
    }
}
