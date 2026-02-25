//! Task runner: iterates parsed tasks and invokes phases.
//!
//! SP-1.6: run Phase 1 for every task in order, capture plan text, log
//! results.  Fail fast on the first task error (no state, no resume).

use std::path::Path;
use std::time::Instant;

use tracing::{error, info};

use crate::config::PealConfig;
use crate::error::PealError;
use crate::phase::{self, PhaseOutput};
use crate::plan::ParsedPlan;

/// The result of running Phase 1 for a single task.
#[derive(Debug, Clone)]
pub struct TaskPhase1Result {
    pub task_index: u32,
    pub plan_text: String,
}

/// Run Phase 1 (plan creation) for every task in order.
///
/// On the first task failure the function returns the error immediately
/// (no partial results, no state persistence).  Each successful invocation
/// is logged with task index, duration, and plan-text length.
pub fn run_phase1_all(
    agent_path: &Path,
    config: &PealConfig,
    plan: &ParsedPlan,
) -> Result<Vec<TaskPhase1Result>, PealError> {
    let task_count = plan.tasks.len();
    info!(task_count, "starting phase 1 for all tasks");

    let mut results: Vec<TaskPhase1Result> = Vec::with_capacity(task_count);

    for (i, task) in plan.tasks.iter().enumerate() {
        let position = i + 1;
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

        let results = run_phase1_all(&echo, &config, &plan).unwrap();

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

        let plan = make_plan(vec![]);
        let results = run_phase1_all(&echo, &config, &plan).unwrap();

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

        let err = run_phase1_all(&false_path, &config, &plan).unwrap_err();

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

        let plan = make_plan(vec![Task {
            index: 42,
            content: "The only task.".to_owned(),
            parallel: false,
        }]);

        let results = run_phase1_all(&echo, &config, &plan).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].task_index, 42);
        assert!(results[0].plan_text.contains("The only task."));
    }

    #[test]
    fn preserves_task_order() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();

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

        let results = run_phase1_all(&echo, &config, &plan).unwrap();

        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![10, 20, 30]);
    }
}
