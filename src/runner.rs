//! Task runner: iterates parsed tasks and invokes phases.
//!
//! SP-1.6: run Phase 1 for every task in order, capture plan text, log results.
//! SP-2.2: sequential runner — Phase 1 → Phase 2 per task, fail-fast.

use std::path::Path;
use std::time::Instant;

use tracing::{error, info};

use crate::config::PealConfig;
use crate::error::PealError;
use crate::phase::{self, PhaseOutput};
use crate::plan::ParsedPlan;
use crate::state::{self, PealState};

/// The result of running Phase 1 for a single task.
#[derive(Debug, Clone)]
pub struct TaskPhase1Result {
    pub task_index: u32,
    pub plan_text: String,
}

/// The result of running both phases for a single task.
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub task_index: u32,
    pub plan_text: String,
    pub phase2_stdout: String,
}

/// Run Phase 1 (plan creation) for every task in order.
///
/// On the first task failure, best-effort saves state then returns the
/// error.  Each successful invocation is logged with task index, duration,
/// and plan-text length.
pub fn run_phase1_all(
    agent_path: &Path,
    config: &PealConfig,
    plan: &ParsedPlan,
    peal_state: &mut PealState,
    state_dir: &Path,
) -> Result<Vec<TaskPhase1Result>, PealError> {
    let task_count = plan.tasks.len();
    info!(task_count, "starting phase 1 for all tasks");

    let mut results: Vec<TaskPhase1Result> = Vec::with_capacity(task_count);

    for (i, task) in plan.tasks.iter().enumerate() {
        let position = i + 1;

        if peal_state.is_task_completed(task.index) {
            info!(
                task_index = task.index,
                position,
                task_count,
                "skipping already-completed task {position}/{task_count}"
            );
            continue;
        }

        info!(
            task_index = task.index,
            position,
            task_count,
            "phase 1: task {position}/{task_count}"
        );

        let start = Instant::now();

        let output: PhaseOutput =
            phase::run_phase1(agent_path, config, task.index, &task.content).map_err(|e| {
                error!(
                    task_index = task.index,
                    position,
                    task_count,
                    err = %e,
                    "phase 1 failed"
                );
                if let Err(save_err) = state::save_state(peal_state, state_dir) {
                    error!(err = %save_err, "failed to save state after phase 1 failure");
                }
                e
            })?;

        let duration = start.elapsed();

        info!(
            task_index = task.index,
            position,
            task_count,
            duration_ms = duration.as_millis() as u64,
            plan_text_len = output.stdout.len(),
            "phase 1 complete"
        );

        peal_state.mark_task_completed(task.index);
        state::save_state(peal_state, state_dir)?;

        results.push(TaskPhase1Result {
            task_index: task.index,
            plan_text: output.stdout,
        });
    }

    info!(
        completed = results.len(),
        task_count, "all phase 1 invocations complete"
    );

    Ok(results)
}

/// Run the full sequential pipeline (Phase 1 → Phase 2) for every task.
///
/// For each task in order:
///   1. Phase 1 (plan creation) — capture the plan text.
///   2. Phase 2 (execution)     — pass the plan text to the agent.
///
/// On success, marks the task completed in `peal_state` and persists to
/// `state_dir`.  On failure, best-effort saves current state before
/// returning the original error.
pub fn run_all(
    agent_path: &Path,
    config: &PealConfig,
    plan: &ParsedPlan,
    peal_state: &mut PealState,
    state_dir: &Path,
) -> Result<Vec<TaskResult>, PealError> {
    let task_count = plan.tasks.len();
    info!(task_count, "starting sequential run (phase 1 → phase 2)");

    let mut results: Vec<TaskResult> = Vec::with_capacity(task_count);

    for (i, task) in plan.tasks.iter().enumerate() {
        let position = i + 1;

        if peal_state.is_task_completed(task.index) {
            info!(
                task_index = task.index,
                position,
                task_count,
                "skipping already-completed task {position}/{task_count}"
            );
            continue;
        }

        // -- Phase 1 --
        info!(
            task_index = task.index,
            position,
            task_count,
            "phase 1: task {position}/{task_count}"
        );

        let p1_start = Instant::now();

        let p1_output: PhaseOutput =
            phase::run_phase1(agent_path, config, task.index, &task.content).map_err(|e| {
                error!(
                    task_index = task.index,
                    position,
                    task_count,
                    err = %e,
                    "phase 1 failed"
                );
                if let Err(save_err) = state::save_state(peal_state, state_dir) {
                    error!(err = %save_err, "failed to save state after phase 1 failure");
                }
                e
            })?;

        let p1_duration = p1_start.elapsed();

        info!(
            task_index = task.index,
            position,
            task_count,
            duration_ms = p1_duration.as_millis() as u64,
            plan_text_len = p1_output.stdout.len(),
            "phase 1 complete"
        );

        // -- Phase 2 --
        info!(
            task_index = task.index,
            position,
            task_count,
            "phase 2: task {position}/{task_count}"
        );

        let p2_start = Instant::now();

        let p2_output: PhaseOutput =
            phase::run_phase2(agent_path, config, task.index, &p1_output.stdout).map_err(|e| {
                error!(
                    task_index = task.index,
                    position,
                    task_count,
                    err = %e,
                    "phase 2 failed"
                );
                if let Err(save_err) = state::save_state(peal_state, state_dir) {
                    error!(err = %save_err, "failed to save state after phase 2 failure");
                }
                e
            })?;

        let p2_duration = p2_start.elapsed();

        info!(
            task_index = task.index,
            position,
            task_count,
            duration_ms = p2_duration.as_millis() as u64,
            stdout_len = p2_output.stdout.len(),
            "phase 2 complete"
        );

        peal_state.mark_task_completed(task.index);
        state::save_state(peal_state, state_dir)?;

        results.push(TaskResult {
            task_index: task.index,
            plan_text: p1_output.stdout,
            phase2_stdout: p2_output.stdout,
        });
    }

    info!(
        completed = results.len(),
        task_count, "all tasks complete (phase 1 → phase 2)"
    );

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PealConfig;
    use crate::plan::{ParsedPlan, Segment, Task};
    use std::path::PathBuf;

    fn test_config(repo: &Path) -> PealConfig {
        PealConfig {
            agent_cmd: "echo".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: repo.to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
        }
    }

    fn fresh_state() -> PealState {
        PealState::new(PathBuf::from("plan.md"), PathBuf::from("/repo"))
    }

    fn make_plan(tasks: Vec<Task>) -> ParsedPlan {
        let segments = tasks
            .iter()
            .map(|t| Segment::Sequential(t.index))
            .collect();
        ParsedPlan { tasks, segments }
    }

    fn resolve_echo() -> PathBuf {
        crate::cursor::resolve_agent_cmd("echo").expect("echo must exist")
    }

    #[test]
    fn runs_phase1_for_all_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task {
                index: 1,
                content: "First task.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "Second task.".to_owned(),
                parallel: false,
            },
            Task {
                index: 3,
                content: "Third task.".to_owned(),
                parallel: false,
            },
        ]);

        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].task_index, 1);
        assert_eq!(results[1].task_index, 2);
        assert_eq!(results[2].task_index, 3);

        for r in &results {
            assert!(
                r.plan_text.contains("Create a plan for implementing this task:"),
                "plan text should contain the prompt (echoed back): {:?}",
                r.plan_text
            );
        }
    }

    #[test]
    fn empty_plan_returns_empty_results() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![]);
        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        assert!(results.is_empty());
    }

    #[test]
    fn fails_fast_on_first_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "false".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
        };

        let false_path =
            crate::cursor::resolve_agent_cmd("false").expect("false must exist");
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task {
                index: 1,
                content: "Will fail.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "Should not run.".to_owned(),
                parallel: false,
            },
        ]);

        let err = run_phase1_all(&false_path, &config, &plan, &mut state, &state_dir).unwrap_err();

        match err {
            PealError::PhaseNonZeroExit { phase, .. } => assert_eq!(phase, 1),
            other => panic!("expected PhaseNonZeroExit, got: {other:?}"),
        }
    }

    #[test]
    fn single_task_plan() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![Task {
            index: 42,
            content: "The only task.".to_owned(),
            parallel: false,
        }]);

        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].task_index, 42);
        assert!(results[0].plan_text.contains("The only task."));
    }

    #[test]
    fn preserves_task_order() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task {
                index: 10,
                content: "Ten.".to_owned(),
                parallel: false,
            },
            Task {
                index: 20,
                content: "Twenty.".to_owned(),
                parallel: false,
            },
            Task {
                index: 30,
                content: "Thirty.".to_owned(),
                parallel: false,
            },
        ]);

        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![10, 20, 30]);
    }

    // -- run_all (SP-2.2) tests --

    #[test]
    fn run_all_executes_both_phases_for_all_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task {
                index: 1,
                content: "First task.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "Second task.".to_owned(),
                parallel: false,
            },
        ]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].task_index, 1);
        assert_eq!(results[1].task_index, 2);

        for r in &results {
            assert!(
                !r.plan_text.is_empty(),
                "plan_text should be non-empty for task {}",
                r.task_index
            );
            assert!(
                !r.phase2_stdout.is_empty(),
                "phase2_stdout should be non-empty for task {}",
                r.task_index
            );
        }
    }

    #[test]
    fn run_all_empty_plan() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![]);
        let results = run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        assert!(results.is_empty());
    }

    #[test]
    fn run_all_fails_fast_on_phase1_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = PealConfig {
            agent_cmd: "false".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
        };

        let false_path =
            crate::cursor::resolve_agent_cmd("false").expect("false must exist");
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task {
                index: 1,
                content: "Will fail in phase 1.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "Should not run.".to_owned(),
                parallel: false,
            },
        ]);

        let err = run_all(&false_path, &config, &plan, &mut state, &state_dir).unwrap_err();

        match err {
            PealError::PhaseNonZeroExit { phase, .. } => assert_eq!(phase, 1),
            other => panic!("expected PhaseNonZeroExit for phase 1, got: {other:?}"),
        }
    }

    #[test]
    fn run_all_single_task() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![Task {
            index: 7,
            content: "The only task.".to_owned(),
            parallel: false,
        }]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].task_index, 7);
        assert!(results[0].plan_text.contains("The only task."));
        assert!(results[0].phase2_stdout.contains("Execute the following plan"));
    }

    #[test]
    fn run_all_preserves_task_order() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task {
                index: 10,
                content: "Ten.".to_owned(),
                parallel: false,
            },
            Task {
                index: 20,
                content: "Twenty.".to_owned(),
                parallel: false,
            },
            Task {
                index: 30,
                content: "Thirty.".to_owned(),
                parallel: false,
            },
        ]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![10, 20, 30]);
    }

    #[test]
    fn run_all_phase2_receives_phase1_plan_text() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![Task {
            index: 1,
            content: "Build a widget.".to_owned(),
            parallel: false,
        }]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        assert!(
            results[0]
                .phase2_stdout
                .contains("---PLAN---"),
            "phase 2 output should contain plan delimiters: {:?}",
            results[0].phase2_stdout
        );
    }

    // -- State persistence integration tests --

    #[test]
    fn run_all_persists_state_with_all_task_indices() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: false },
            Task { index: 3, content: "C.".to_owned(), parallel: false },
        ]);

        run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        let loaded = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist after run");
        assert_eq!(loaded.completed_task_indices, vec![1, 2, 3]);
    }

    #[test]
    fn run_all_on_failure_persists_only_completed_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join(".peal");

        let echo = resolve_echo();
        let false_path =
            crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        // Run first task successfully with echo, then simulate failure.
        // We can't mix agents mid-run, so we use `false` for all tasks
        // and expect zero completed tasks in state.
        let config = PealConfig {
            agent_cmd: "false".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            state_dir: state_dir.clone(),
            phase_timeout_sec: 30,
            parallel: false,
            max_parallel: 4,
            log_level: None,
            log_file: None,
        };

        let mut state = fresh_state();
        let plan = make_plan(vec![
            Task { index: 1, content: "Will fail.".to_owned(), parallel: false },
            Task { index: 2, content: "Never reached.".to_owned(), parallel: false },
        ]);

        let _ = run_all(&false_path, &config, &plan, &mut state, &state_dir);

        let loaded = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist even on failure");
        assert!(
            loaded.completed_task_indices.is_empty(),
            "no tasks should be completed since first task failed"
        );

        // Now run successfully with echo to verify partial completion.
        let good_config = test_config(dir.path());
        let plan2 = make_plan(vec![
            Task { index: 10, content: "A.".to_owned(), parallel: false },
            Task { index: 20, content: "B.".to_owned(), parallel: false },
        ]);

        let mut state2 = fresh_state();
        run_all(&echo, &good_config, &plan2, &mut state2, &state_dir).unwrap();

        let loaded2 = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist");
        assert_eq!(loaded2.completed_task_indices, vec![10, 20]);
    }

    #[test]
    fn run_all_creates_state_dir_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join("nested").join(".peal");
        let mut state = fresh_state();

        assert!(!state_dir.exists());

        let plan = make_plan(vec![
            Task { index: 1, content: "X.".to_owned(), parallel: false },
        ]);

        run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        assert!(state_dir.join("state.json").exists());
    }

    // -- Resume semantics tests (SP-3.3) --

    #[test]
    fn run_all_skips_previously_completed_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");

        let mut state = fresh_state();
        state.mark_task_completed(1);

        let plan = make_plan(vec![
            Task { index: 1, content: "Already done.".to_owned(), parallel: false },
            Task { index: 2, content: "Still pending.".to_owned(), parallel: false },
            Task { index: 3, content: "Also pending.".to_owned(), parallel: false },
        ]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![2, 3], "task 1 should be skipped");
    }

    #[test]
    fn run_phase1_all_skips_previously_completed_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");

        let mut state = fresh_state();
        state.mark_task_completed(1);
        state.mark_task_completed(2);

        let plan = make_plan(vec![
            Task { index: 1, content: "Done.".to_owned(), parallel: false },
            Task { index: 2, content: "Done.".to_owned(), parallel: false },
            Task { index: 3, content: "Pending.".to_owned(), parallel: false },
        ]);

        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![3], "tasks 1 and 2 should be skipped");
    }

    #[test]
    fn run_all_all_completed_produces_empty_results() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");

        let mut state = fresh_state();
        state.mark_task_completed(1);
        state.mark_task_completed(2);

        let plan = make_plan(vec![
            Task { index: 1, content: "Done.".to_owned(), parallel: false },
            Task { index: 2, content: "Done.".to_owned(), parallel: false },
        ]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        assert!(results.is_empty(), "no tasks should run when all are completed");
    }

    #[test]
    fn run_all_partial_completion_resumes_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");

        // Simulate a previous run that completed tasks 1 and 2.
        let mut state = fresh_state();
        state.mark_task_completed(1);
        state.mark_task_completed(2);
        state::save_state(&state, &state_dir).unwrap();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: false },
            Task { index: 3, content: "C.".to_owned(), parallel: false },
            Task { index: 4, content: "D.".to_owned(), parallel: false },
        ]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir).unwrap();

        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![3, 4], "should resume from task 3");

        // Verify state now has all tasks completed.
        let loaded = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist");
        assert_eq!(loaded.completed_task_indices, vec![1, 2, 3, 4]);
    }
}
