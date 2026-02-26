//! Task runner: iterates parsed tasks and invokes phases.
//!
//! SP-1.6: run Phase 1 for every task in order, capture plan text, log results.
//! SP-2.2: sequential runner — Phase 1 → Phase 2 per task, fail-fast.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use tracing::{error, info, warn};

use crate::config::PealConfig;
use crate::error::PealError;
use crate::phase::{self, PhaseOutput};
use crate::plan::ParsedPlan;
use crate::state::{self, PealState};
use crate::stet;

/// Run stet review with on_stet_fail policy. Returns Ok(Some(result)) on success,
/// Ok(None) when policy is "skip" and run failed, Err when policy is "fail" or "retry_once" exhausted.
fn run_stet_review_with_policy(
    stet_path: &Path,
    repo_path: &Path,
    extra_args: &[String],
    timeout: Option<Duration>,
    on_stet_fail: &str,
    task_index: u32,
    peal_state: &mut PealState,
    state_dir: &Path,
) -> Result<Option<stet::StetRunResult>, PealError> {
    let first = stet::run_review(stet_path, repo_path, extra_args, timeout);
    match first {
        Ok(r) => return Ok(Some(r)),
        Err(e) => {
            if on_stet_fail == "retry_once" {
                warn!(task_index, err = %e, "stet run failed, retrying once");
                match stet::run_review(stet_path, repo_path, extra_args, timeout) {
                    Ok(r) => return Ok(Some(r)),
                    Err(e2) => {
                        error!(task_index, err = %e2, "stet run failed");
                        if let Err(save_err) = state::save_state(peal_state, state_dir) {
                            error!(err = %save_err, "failed to save state after stet failure");
                        }
                        return Err(e2);
                    }
                }
            }
            if on_stet_fail == "skip" {
                warn!(task_index, err = %e, "stet phase skipped");
                return Ok(None);
            }
            error!(task_index, err = %e, "stet run failed");
            if let Err(save_err) = state::save_state(peal_state, state_dir) {
                error!(err = %save_err, "failed to save state after stet failure");
            }
            return Err(e);
        }
    }
}

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
    pub phase3_outcome: Option<stet::AddressLoopOutcome>,
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
    stet_path: Option<&Path>,
) -> Result<Vec<TaskPhase1Result>, PealError> {
    let task_count = plan.tasks.len();
    let phase3_available = stet_path.is_some();
    info!(task_count, phase3_available, "starting phase 1 for all tasks");

    let mut results: Vec<TaskPhase1Result> = Vec::with_capacity(task_count);

    for (i, task) in plan.tasks.iter().enumerate() {
        let position = i + 1;

        if peal_state.is_task_completed(task.index) {
            info!(
                task_index = task.index,
                position, task_count, "skipping already-completed task {position}/{task_count}"
            );
            continue;
        }

        info!(
            task_index = task.index,
            position, task_count, "phase 1: task {position}/{task_count}"
        );

        let start = Instant::now();

        let output: PhaseOutput = phase::run_phase1(agent_path, config, task.index, &task.content)
            .map_err(|e| {
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

/// Run Phase 1 → Phase 2 → Phase 3 for a single task, mark it completed, and
/// persist state. Extracted from the per-task body so both sequential and
/// parallel segment branches can reuse it.
fn run_single_task(
    agent_path: &Path,
    config: &PealConfig,
    task: &crate::plan::Task,
    peal_state: &mut PealState,
    state_dir: &Path,
    stet_path: Option<&Path>,
    task_count: usize,
    position: usize,
) -> Result<TaskResult, PealError> {
    // -- Phase 1 --
    info!(
        task_index = task.index,
        position, task_count, "phase 1: task {position}/{task_count}"
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
        position, task_count, "phase 2: task {position}/{task_count}"
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

    // -- Phase 3 (stet review + address) --
    let phase3_outcome = if let Some(sp) = stet_path {
        let timeout = Some(Duration::from_secs(config.phase_timeout_sec));

        info!(
            task_index = task.index,
            position, task_count,
            "phase 3: running stet review"
        );

        let stet_result = run_stet_review_with_policy(
            sp,
            &config.repo_path,
            &config.stet_run_extra_args,
            timeout,
            config.on_stet_fail.as_str(),
            task.index,
            peal_state,
            state_dir,
        )?;

        let stet_result = match stet_result {
            Some(r) => r,
            None => {
                // skip: stet run failed, phase 3 skipped for this task
                info!(task_index = task.index, "phase 3 skipped (stet run failed)");
                peal_state.mark_task_completed(task.index);
                state::save_state(peal_state, state_dir)?;
                return Ok(TaskResult {
                    task_index: task.index,
                    plan_text: p1_output.stdout,
                    phase2_stdout: p2_output.stdout,
                    phase3_outcome: None,
                });
            }
        };

        if stet_result.has_findings {
            info!(task_index = task.index, "phase 3: findings detected, starting address loop");

            let outcome = match stet::address_loop(agent_path, sp, config, task.index, &stet_result) {
                Ok(o) => o,
                Err(e) => {
                    if config.on_stet_fail == "retry_once" {
                        warn!(task_index = task.index, err = %e, "address loop failed, retrying once");
                        match stet::address_loop(agent_path, sp, config, task.index, &stet_result) {
                            Ok(o) => o,
                            Err(e2) => {
                                error!(task_index = task.index, err = %e2, "address loop failed after retry");
                                if let Err(save_err) = state::save_state(peal_state, state_dir) {
                                    error!(err = %save_err, "failed to save state after address failure");
                                }
                                return Err(e2);
                            }
                        }
                    } else if config.on_stet_fail == "skip" {
                        warn!(task_index = task.index, err = %e, "stet phase skipped");
                        stet::AddressLoopOutcome {
                            rounds_used: 0,
                            findings_resolved: false,
                            last_stet_result: stet_result.clone(),
                        }
                    } else {
                        error!(task_index = task.index, err = %e, "address loop failed");
                        if let Err(save_err) = state::save_state(peal_state, state_dir) {
                            error!(err = %save_err, "failed to save state after address failure");
                        }
                        return Err(e);
                    }
                }
            };

            info!(
                task_index = task.index,
                rounds = outcome.rounds_used,
                resolved = outcome.findings_resolved,
                "phase 3 complete"
            );
            Some(outcome)
        } else {
            info!(task_index = task.index, "phase 3: no findings, skipping address loop");
            Some(stet::AddressLoopOutcome {
                rounds_used: 0,
                findings_resolved: true,
                last_stet_result: stet_result,
            })
        }
    } else {
        None
    };

    peal_state.mark_task_completed(task.index);
    state::save_state(peal_state, state_dir)?;

    Ok(TaskResult {
        task_index: task.index,
        plan_text: p1_output.stdout,
        phase2_stdout: p2_output.stdout,
        phase3_outcome,
    })
}

/// Run Phase 1 → Phase 2 for a single task with no state mutation and no Phase 3.
/// Each scoped thread executes this; the main thread handles state and Phase 3 after join.
fn run_phases_1_2(
    agent_path: &Path,
    config: &PealConfig,
    task: &crate::plan::Task,
    task_count: usize,
    position: usize,
) -> Result<(String, String), PealError> {
    info!(
        task_index = task.index,
        position, task_count, "phase 1: task {position}/{task_count}"
    );

    let p1_start = Instant::now();
    let p1_output =
        phase::run_phase1(agent_path, config, task.index, &task.content).map_err(|e| {
            error!(
                task_index = task.index,
                position, task_count, err = %e, "phase 1 failed"
            );
            e
        })?;
    let p1_duration = p1_start.elapsed();

    info!(
        task_index = task.index,
        position, task_count,
        duration_ms = p1_duration.as_millis() as u64,
        plan_text_len = p1_output.stdout.len(),
        "phase 1 complete"
    );

    info!(
        task_index = task.index,
        position, task_count, "phase 2: task {position}/{task_count}"
    );

    let p2_start = Instant::now();
    let p2_output =
        phase::run_phase2(agent_path, config, task.index, &p1_output.stdout).map_err(|e| {
            error!(
                task_index = task.index,
                position, task_count, err = %e, "phase 2 failed"
            );
            e
        })?;
    let p2_duration = p2_start.elapsed();

    info!(
        task_index = task.index,
        position, task_count,
        duration_ms = p2_duration.as_millis() as u64,
        stdout_len = p2_output.stdout.len(),
        "phase 2 complete"
    );

    Ok((p1_output.stdout, p2_output.stdout))
}

/// Run Phase 1 → Phase 2 concurrently for a batch of pending tasks.
///
/// One execution stream per task (Phase 1 → Phase 2); streams are joined before
/// the caller runs Phase 3; no shared mutable state.
///
/// Tasks are chunked into groups of `max_concurrent`; within each chunk,
/// scoped threads run one task each. After all threads in a chunk join,
/// results are partitioned into successes and failures. Processing stops
/// after the first chunk that contains any failure.
fn run_parallel_block(
    agent_path: &Path,
    config: &PealConfig,
    plan: &ParsedPlan,
    pending: &[u32],
    task_count: usize,
    base_position: usize,
    max_concurrent: usize,
) -> (Vec<(u32, String, String)>, Vec<(u32, PealError)>) {
    let mut successes: Vec<(u32, String, String)> = Vec::new();
    let mut failures: Vec<(u32, PealError)> = Vec::new();
    let mut offset = 0;

    for chunk in pending.chunks(max_concurrent) {
        std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .enumerate()
                .map(|(i, &idx)| {
                    let position = base_position + offset + i + 1;
                    let task = plan
                        .task_by_index(idx)
                        .expect("task index validated before parallel block");
                    s.spawn(move || {
                        run_phases_1_2(agent_path, config, task, task_count, position)
                            .map(|(plan_text, p2_stdout)| (idx, plan_text, p2_stdout))
                            .map_err(|e| (idx, e))
                    })
                })
                .collect();

            for handle in handles {
                match handle.join().expect("scoped thread must not panic") {
                    Ok(success) => successes.push(success),
                    Err(failure) => failures.push(failure),
                }
            }
        });

        offset += chunk.len();

        if !failures.is_empty() {
            break;
        }
    }

    (successes, failures)
}

/// Segment-aware execution scheduler (SP-5.1).
///
/// Execution order is derived solely from `plan.execution_schedule()` (i.e. `plan.segments`):
/// segment 1, then 2, … (sequential segments and parallel blocks in sequence).
/// - `Sequential(idx)` — runs the single task through P1 → P2 → P3.
/// - `Parallel(indices)` — when `config.parallel` is true and more than one
///   task is pending, runs P1 → P2 concurrently via `std::thread::scope`,
///   persists successful tasks, then runs P3 sequentially.  Falls back to
///   sequential execution when `config.parallel` is false or only one task
///   remains.
///
/// State is persisted per-task (not per-segment) to enable fine-grained resume.
pub fn run_scheduled(
    agent_path: &Path,
    config: &PealConfig,
    plan: &ParsedPlan,
    peal_state: &mut PealState,
    state_dir: &Path,
    stet_path: Option<&Path>,
) -> Result<Vec<TaskResult>, PealError> {
    let task_count = plan.tasks.len();
    let phase3_available = stet_path.is_some();
    let schedule = plan.execution_schedule();
    info!(
        task_count,
        segment_count = schedule.len(),
        phase3_available,
        "starting scheduled run"
    );

    let mut results: Vec<TaskResult> = Vec::with_capacity(task_count);
    let mut position: usize = 0;

    for segment in schedule {
        match segment {
            crate::plan::Segment::Sequential(idx) => {
                let idx = *idx;
                position += 1;

                if peal_state.is_task_completed(idx) {
                    info!(
                        task_index = idx,
                        position, task_count, "skipping already-completed task"
                    );
                    continue;
                }

                let task = plan.task_by_index(idx).ok_or_else(|| {
                    PealError::TaskNotFound {
                        index: idx,
                        available: plan.tasks.iter().map(|t| t.index).collect(),
                    }
                })?;

                let result = run_single_task(
                    agent_path, config, task, peal_state, state_dir, stet_path,
                    task_count, position,
                )?;
                results.push(result);
            }

            crate::plan::Segment::Parallel(indices) => {
                let pending: Vec<u32> = indices
                    .iter()
                    .copied()
                    .filter(|idx| !peal_state.is_task_completed(*idx))
                    .collect();

                if pending.is_empty() {
                    position += indices.len();
                    info!(
                        block_indices = ?indices,
                        "skipping fully-completed parallel block"
                    );
                    continue;
                }

                if config.parallel && pending.len() > 1 && config.max_parallel > 1 {
                    // -- Concurrent path (SP-5.2). All Phase 2s in the block complete before any Phase 3 runs. --
                    let max_concurrent = (config.max_parallel as usize).max(1);

                    info!(
                        block_size = indices.len(),
                        pending_count = pending.len(),
                        max_concurrent,
                        block_indices = ?indices,
                        "parallel block: running concurrently"
                    );

                    let (successes, failures) = run_parallel_block(
                        agent_path, config, plan, &pending,
                        task_count, position, max_concurrent,
                    );

                    // Persist all successful P1+P2 completions before Phase 3.
                    for &(idx, _, _) in &successes {
                        peal_state.mark_task_completed(idx);
                    }
                    if !successes.is_empty() {
                        state::save_state(peal_state, state_dir)?;
                    }

                    // Phase 3 sequentially in block task order (segment indices order).
                    let mut successes_by_index: HashMap<u32, (String, String)> = successes
                        .into_iter()
                        .map(|(idx, plan_text, phase2_stdout)| (idx, (plan_text, phase2_stdout)))
                        .collect();

                    let mut phase3_count = 0usize;
                    let mut phase3_continued_after_failure = false;
                    for idx in indices {
                        let Some((plan_text, phase2_stdout)) = successes_by_index.remove(&idx) else {
                            continue;
                        };
                        phase3_count += 1;
                        position += 1;

                        let phase3_result: Result<Option<stet::AddressLoopOutcome>, PealError> =
                            (|| {
                                if let Some(sp) = stet_path {
                                    let timeout = Some(Duration::from_secs(config.phase_timeout_sec));

                                    info!(
                                        task_index = idx,
                                        position, task_count,
                                        "phase 3: running stet review"
                                    );

                                    let stet_result = run_stet_review_with_policy(
                                        sp,
                                        &config.repo_path,
                                        &config.stet_run_extra_args,
                                        timeout,
                                        config.on_stet_fail.as_str(),
                                        idx,
                                        peal_state,
                                        state_dir,
                                    )?;

                                    let stet_result = match stet_result {
                                        Some(r) => r,
                                        None => {
                                            info!(task_index = idx, "phase 3 skipped (stet run failed)");
                                            return Ok(None);
                                        }
                                    };

                                    if stet_result.has_findings {
                                        info!(task_index = idx, "phase 3: findings detected, starting address loop");

                                        let outcome = match stet::address_loop(agent_path, sp, config, idx, &stet_result) {
                                            Ok(o) => o,
                                            Err(e) => {
                                                if config.on_stet_fail == "retry_once" {
                                                    warn!(task_index = idx, err = %e, "address loop failed, retrying once");
                                                    match stet::address_loop(agent_path, sp, config, idx, &stet_result) {
                                                        Ok(o) => o,
                                                        Err(e2) => {
                                                            error!(task_index = idx, err = %e2, "address loop failed after retry");
                                                            if let Err(save_err) = state::save_state(peal_state, state_dir) {
                                                                error!(err = %save_err, "failed to save state after address failure");
                                                            }
                                                            return Err(e2);
                                                        }
                                                    }
                                                } else if config.on_stet_fail == "skip" {
                                                    warn!(task_index = idx, err = %e, "stet phase skipped");
                                                    stet::AddressLoopOutcome {
                                                        rounds_used: 0,
                                                        findings_resolved: false,
                                                        last_stet_result: stet_result.clone(),
                                                    }
                                                } else {
                                                    error!(task_index = idx, err = %e, "address loop failed");
                                                    if let Err(save_err) = state::save_state(peal_state, state_dir) {
                                                        error!(err = %save_err, "failed to save state after address failure");
                                                    }
                                                    return Err(e);
                                                }
                                            }
                                        };

                                        info!(
                                            task_index = idx,
                                            rounds = outcome.rounds_used,
                                            resolved = outcome.findings_resolved,
                                            "phase 3 complete"
                                        );
                                        Ok(Some(Some(outcome)))
                                    } else {
                                        info!(task_index = idx, "phase 3: no findings, skipping address loop");
                                        Ok(Some(Some(stet::AddressLoopOutcome {
                                            rounds_used: 0,
                                            findings_resolved: true,
                                            last_stet_result: stet_result,
                                        })))
                                    }
                                } else {
                                    Ok(None)
                                }
                            })();

                        match phase3_result {
                            Ok(phase3_outcome) => {
                                results.push(TaskResult {
                                    task_index: idx,
                                    plan_text,
                                    phase2_stdout,
                                    phase3_outcome,
                                });
                            }
                            Err(e) => {
                                if config.continue_with_remaining_tasks {
                                    warn!(
                                        task_index = idx,
                                        err = %e,
                                        "phase 3 failed in parallel block; continuing with remaining segments"
                                    );
                                    if let Err(save_err) = state::save_state(peal_state, state_dir) {
                                        error!(err = %save_err, "failed to save state after phase 3 failure");
                                    }
                                    position += indices.len() - phase3_count;
                                    phase3_continued_after_failure = true;
                                    break;
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                    }

                    let completed_in_block = indices.len() - pending.len();
                    if !phase3_continued_after_failure {
                        position += completed_in_block + failures.len();
                    }

                    if !failures.is_empty() {
                        for (fail_idx, err) in &failures {
                            warn!(
                                task_index = fail_idx,
                                err = %err,
                                "task failed in parallel block (P1/P2)"
                            );
                        }
                        if !config.continue_with_remaining_tasks {
                            if let Some((_idx, err)) = failures.into_iter().next() {
                                return Err(err);
                            }
                        }
                    }
                } else {
                    // -- Sequential fallback --
                    info!(
                        block_size = indices.len(),
                        pending_count = pending.len(),
                        block_indices = ?indices,
                        "parallel block (sequential fallback)"
                    );

                    for idx in &pending {
                        position += 1;

                        let task = plan.task_by_index(*idx).ok_or_else(|| {
                            PealError::TaskNotFound {
                                index: *idx,
                                available: plan.tasks.iter().map(|t| t.index).collect(),
                            }
                        })?;

                        let result = run_single_task(
                            agent_path, config, task, peal_state, state_dir, stet_path,
                            task_count, position,
                        )?;
                        results.push(result);
                    }

                    let completed_in_block = indices.len() - pending.len();
                    position += completed_in_block;
                }
            }
        }
    }

    info!(
        completed = results.len(),
        task_count, "all tasks complete"
    );

    Ok(results)
}

/// Run the full pipeline for every task. Delegates to `run_scheduled`.
///
/// Kept as a stable entry point so existing callers and tests continue to work.
pub fn run_all(
    agent_path: &Path,
    config: &PealConfig,
    plan: &ParsedPlan,
    peal_state: &mut PealState,
    state_dir: &Path,
    stet_path: Option<&Path>,
) -> Result<Vec<TaskResult>, PealError> {
    run_scheduled(agent_path, config, plan, peal_state, state_dir, stet_path)
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
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
        }
    }

    fn fresh_state() -> PealState {
        PealState::new(PathBuf::from("plan.md"), PathBuf::from("/repo"))
    }

    fn make_plan(tasks: Vec<Task>) -> ParsedPlan {
        let segments = crate::plan::compute_segments(&tasks);
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

        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].task_index, 1);
        assert_eq!(results[1].task_index, 2);
        assert_eq!(results[2].task_index, 3);

        for r in &results {
            assert!(
                r.plan_text
                    .contains("Create a plan for implementing this task:"),
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
        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
        };

        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");
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

        let err = run_phase1_all(&false_path, &config, &plan, &mut state, &state_dir, None).unwrap_err();

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

        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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

        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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
            assert!(
                r.phase3_outcome.is_none(),
                "phase3_outcome should be None when stet_path is None for task {}",
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
        let results = run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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
            on_findings_remaining: "fail".to_owned(),
            state_dir: PathBuf::from(".peal"),
            phase_timeout_sec: 30,
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
        };

        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");
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

        let err = run_all(&false_path, &config, &plan, &mut state, &state_dir, None).unwrap_err();

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

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].task_index, 7);
        assert!(results[0].plan_text.contains("The only task."));
        assert!(
            results[0]
                .phase2_stdout
                .contains("Execute the following plan")
        );
        assert!(results[0].phase3_outcome.is_none());
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

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        assert!(
            results[0].phase2_stdout.contains("---PLAN---"),
            "phase 2 output should contain plan delimiters: {:?}",
            results[0].phase2_stdout
        );
        assert!(results[0].phase3_outcome.is_none());
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
            Task {
                index: 1,
                content: "A.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "B.".to_owned(),
                parallel: false,
            },
            Task {
                index: 3,
                content: "C.".to_owned(),
                parallel: false,
            },
        ]);

        run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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
        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");

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
            on_findings_remaining: "fail".to_owned(),
            state_dir: state_dir.clone(),
            phase_timeout_sec: 30,
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
        };

        let mut state = fresh_state();
        let plan = make_plan(vec![
            Task {
                index: 1,
                content: "Will fail.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "Never reached.".to_owned(),
                parallel: false,
            },
        ]);

        let _ = run_all(&false_path, &config, &plan, &mut state, &state_dir, None);

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
            Task {
                index: 10,
                content: "A.".to_owned(),
                parallel: false,
            },
            Task {
                index: 20,
                content: "B.".to_owned(),
                parallel: false,
            },
        ]);

        let mut state2 = fresh_state();
        run_all(&echo, &good_config, &plan2, &mut state2, &state_dir, None).unwrap();

        let loaded2 = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist");
        assert_eq!(loaded2.completed_task_indices, vec![10, 20]);
    }

    /// SP-5.4: max_parallel == 1 forces sequential fallback for a parallel block.
    #[test]
    fn max_parallel_eq_1_uses_sequential_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config_parallel(dir.path());
        config.max_parallel = 1;
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: true },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
        ]);

        assert_eq!(plan.segments, vec![Segment::Parallel(vec![1, 2, 3])]);

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();
        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![1, 2, 3], "max_parallel=1 runs block sequentially in order");
    }

    /// SP-5.4: continue_with_remaining_tasks=true → one failure in block persists completed, returns Ok.
    #[test]
    fn continue_with_remaining_tasks_true_returns_ok() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join(".peal");
        let echo = resolve_echo();
        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        // Complete task 1 first.
        let mut config = test_config_parallel(dir.path());
        let mut state = fresh_state();
        let plan1 = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
        ]);
        run_scheduled(&echo, &config, &plan1, &mut state, &state_dir, None).unwrap();
        assert!(state.is_task_completed(1));

        // Run with parallel block (2, 3) that will fail; continue_with_remaining_tasks = true.
        config.agent_cmd = "false".to_owned();
        config.continue_with_remaining_tasks = true;
        let plan2 = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
        ]);

        let result = run_scheduled(&false_path, &config, &plan2, &mut state, &state_dir, None);
        assert!(result.is_ok(), "continue_with_remaining_tasks=true should return Ok: {:?}", result.err());

        let loaded = state::load_state(&state_dir).unwrap().expect("state file should exist");
        assert!(loaded.is_task_completed(1), "task 1 should remain completed");
        assert!(!loaded.is_task_completed(2));
        assert!(!loaded.is_task_completed(3));
    }

    #[test]
    fn run_all_creates_state_dir_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join("nested").join(".peal");
        let mut state = fresh_state();

        assert!(!state_dir.exists());

        let plan = make_plan(vec![Task {
            index: 1,
            content: "X.".to_owned(),
            parallel: false,
        }]);

        run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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
            Task {
                index: 1,
                content: "Already done.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "Still pending.".to_owned(),
                parallel: false,
            },
            Task {
                index: 3,
                content: "Also pending.".to_owned(),
                parallel: false,
            },
        ]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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
            Task {
                index: 1,
                content: "Done.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "Done.".to_owned(),
                parallel: false,
            },
            Task {
                index: 3,
                content: "Pending.".to_owned(),
                parallel: false,
            },
        ]);

        let results = run_phase1_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

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
            Task {
                index: 1,
                content: "Done.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "Done.".to_owned(),
                parallel: false,
            },
        ]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        assert!(
            results.is_empty(),
            "no tasks should run when all are completed"
        );
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
            Task {
                index: 1,
                content: "A.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "B.".to_owned(),
                parallel: false,
            },
            Task {
                index: 3,
                content: "C.".to_owned(),
                parallel: false,
            },
            Task {
                index: 4,
                content: "D.".to_owned(),
                parallel: false,
            },
        ]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![3, 4], "should resume from task 3");

        // Verify state now has all tasks completed.
        let loaded = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist");
        assert_eq!(loaded.completed_task_indices, vec![1, 2, 3, 4]);
    }

    // -- Phase 3 integration tests (SP-4.6) --

    #[test]
    fn run_all_skips_phase3_when_stet_none() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task {
                index: 1,
                content: "A.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "B.".to_owned(),
                parallel: false,
            },
        ]);

        let results = run_all(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        assert_eq!(results.len(), 2);
        for r in &results {
            assert!(
                r.phase3_outcome.is_none(),
                "phase3_outcome should be None when stet_path is None for task {}",
                r.task_index
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_all_runs_phase3_no_findings() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        // `true` exits 0 with empty stdout → detect_findings returns false.
        let stet = crate::cursor::resolve_agent_cmd("true").expect("true must exist");

        let plan = make_plan(vec![Task {
            index: 1,
            content: "A.".to_owned(),
            parallel: false,
        }]);

        let results =
            run_all(&echo, &config, &plan, &mut state, &state_dir, Some(&stet)).unwrap();

        assert_eq!(results.len(), 1);
        let outcome = results[0]
            .phase3_outcome
            .as_ref()
            .expect("phase3_outcome should be Some when stet_path is provided");
        assert!(outcome.findings_resolved);
        assert_eq!(outcome.rounds_used, 0);
    }

    #[cfg(unix)]
    #[test]
    fn run_all_runs_phase3_with_findings_resolved() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();
        let config = test_config(dir.path());

        // Stateful stet stub: returns findings on first call, clean on second.
        let stet_script = dir.path().join("stet-stub");
        std::fs::write(
            &stet_script,
            "#!/bin/sh\n\
             STATE_FILE=\"$PWD/.stet_state\"\n\
             if [ -f \"$STATE_FILE\" ]; then\n\
             exit 0\n\
             else\n\
             touch \"$STATE_FILE\"\n\
             echo '{\"findings\": [{\"id\": \"f1\", \"message\": \"test\"}]}'\n\
             exit 1\n\
             fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&stet_script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let plan = make_plan(vec![Task {
            index: 1,
            content: "A.".to_owned(),
            parallel: false,
        }]);

        let results =
            run_all(&echo, &config, &plan, &mut state, &state_dir, Some(&stet_script)).unwrap();

        assert_eq!(results.len(), 1);
        let outcome = results[0]
            .phase3_outcome
            .as_ref()
            .expect("phase3_outcome should be Some");
        assert!(outcome.findings_resolved);
        assert!(
            outcome.rounds_used >= 1,
            "should have used at least 1 address round, got {}",
            outcome.rounds_used
        );
    }

    #[test]
    fn run_all_phase3_failure_saves_state() {
        let dir = tempfile::tempdir().unwrap();
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();
        let config = test_config(dir.path());

        let bad_stet = PathBuf::from("/no/such/stet-binary");

        let plan = make_plan(vec![
            Task {
                index: 1,
                content: "A.".to_owned(),
                parallel: false,
            },
            Task {
                index: 2,
                content: "B.".to_owned(),
                parallel: false,
            },
        ]);

        let err =
            run_all(&echo, &config, &plan, &mut state, &state_dir, Some(&bad_stet)).unwrap_err();

        match err {
            PealError::StetRunFailed { .. } => {}
            other => panic!("expected StetRunFailed, got: {other:?}"),
        }

        // State saved before error propagated; task not marked complete because
        // Phase 3 failed before mark_task_completed.
        let loaded = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist even on stet failure");
        assert!(
            loaded.completed_task_indices.is_empty(),
            "no tasks should be completed since stet failed before marking"
        );
    }

    // -- run_scheduled / segment-aware scheduler tests (SP-5.1) --

    #[test]
    fn scheduled_all_sequential_runs_in_order() {
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

        assert_eq!(
            plan.segments,
            vec![Segment::Sequential(1), Segment::Sequential(2), Segment::Sequential(3)]
        );

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();
        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![1, 2, 3]);
    }

    #[test]
    fn scheduled_mixed_plan_runs_all_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
            Task { index: 4, content: "D.".to_owned(), parallel: false },
        ]);

        assert_eq!(
            plan.segments,
            vec![
                Segment::Sequential(1),
                Segment::Parallel(vec![2, 3]),
                Segment::Sequential(4),
            ]
        );

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();
        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![1, 2, 3, 4]);

        assert!(state.is_task_completed(1));
        assert!(state.is_task_completed(2));
        assert!(state.is_task_completed(3));
        assert!(state.is_task_completed(4));
    }

    #[test]
    fn scheduled_resume_within_parallel_block() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");

        let mut state = fresh_state();
        state.mark_task_completed(1);
        state.mark_task_completed(2);

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
            Task { index: 4, content: "D.".to_owned(), parallel: false },
        ]);

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();
        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![3, 4], "task 2 already done; only 3 and 4 should run");
    }

    /// SP-5.5: Resume into parallel block exercises the concurrent path (P1+P2 in parallel, P3 in segment order).
    /// Plan: sequential 1, parallel block (2, 3, 4). Resume with 1 and 2 completed; only 3 and 4 run.
    #[test]
    fn scheduled_resume_parallel_block_concurrent_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config_parallel(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");

        let mut state = fresh_state();
        state.mark_task_completed(1);
        state.mark_task_completed(2);

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
            Task { index: 4, content: "D.".to_owned(), parallel: true },
        ]);

        assert_eq!(
            plan.segments,
            vec![Segment::Sequential(1), Segment::Parallel(vec![2, 3, 4])]
        );

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();
        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![3, 4], "only pending tasks 3 and 4 in block should run; results in segment order");

        let loaded = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist");
        assert_eq!(
            loaded.completed_task_indices,
            vec![1, 2, 3, 4],
            "all four tasks should be completed after run"
        );
    }

    #[test]
    fn scheduled_single_parallel_demoted() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: false },
        ]);

        // compute_segments demotes single-parallel to Sequential.
        assert_eq!(
            plan.segments,
            vec![Segment::Sequential(1), Segment::Sequential(2), Segment::Sequential(3)]
        );

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();
        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![1, 2, 3]);
    }

    #[test]
    fn scheduled_all_completed_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");

        let mut state = fresh_state();
        state.mark_task_completed(1);
        state.mark_task_completed(2);
        state.mark_task_completed(3);

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
        ]);

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();
        assert!(results.is_empty(), "no tasks should run when all are completed");
    }

    #[test]
    fn scheduled_fail_fast_in_parallel_block_saves_prior() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join(".peal");
        let _echo = resolve_echo();
        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        // Task 1 runs with echo (succeeds), then task 2 in the parallel block
        // would need to fail. Since we can't mix agents per-task, we instead
        // test that a fully-failing agent applied to a parallel block still
        // persists the state properly.
        let config = PealConfig {
            agent_cmd: "false".to_owned(),
            plan_path: PathBuf::from("plan.md"),
            repo_path: dir.path().to_path_buf(),
            stet_commands: vec![],
            sandbox: "disabled".to_owned(),
            model: None,
            max_address_rounds: 3,
            on_findings_remaining: "fail".to_owned(),
            state_dir: state_dir.clone(),
            phase_timeout_sec: 30,
            phase_retry_count: 0,
            parallel: false,
            max_parallel: 4,
            continue_with_remaining_tasks: false,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_extra_args: vec![],
            stet_run_extra_args: vec![],
            stet_disable_llm_triage: false,
            stet_dismiss_patterns: vec![],
            on_stet_fail: "fail".to_owned(),
            post_run_commands: vec![],
            post_run_timeout_sec: None,
            normalize_plan: false,
            normalize_retry_count: 0,
            normalize_prompt_path: None,
        };

        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "Will fail.".to_owned(), parallel: true },
            Task { index: 2, content: "Never reached.".to_owned(), parallel: true },
            Task { index: 3, content: "Never reached.".to_owned(), parallel: false },
        ]);

        assert_eq!(
            plan.segments,
            vec![Segment::Parallel(vec![1, 2]), Segment::Sequential(3)]
        );

        let err = run_scheduled(&false_path, &config, &plan, &mut state, &state_dir, None)
            .unwrap_err();

        match err {
            PealError::PhaseNonZeroExit { phase, .. } => assert_eq!(phase, 1),
            other => panic!("expected PhaseNonZeroExit, got: {other:?}"),
        }

        let loaded = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist even on failure");
        assert!(
            loaded.completed_task_indices.is_empty(),
            "no tasks should be completed since first task in block failed"
        );
    }

    // -- SP-5.2 concurrent parallel execution tests --

    fn test_config_parallel(repo: &Path) -> PealConfig {
        PealConfig {
            parallel: true,
            ..test_config(repo)
        }
    }

    #[test]
    fn parallel_block_runs_concurrently() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config_parallel(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: true },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
        ]);

        assert_eq!(plan.segments, vec![Segment::Parallel(vec![1, 2, 3])]);

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        assert_eq!(results.len(), 3);
        let mut indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        indices.sort();
        assert_eq!(indices, vec![1, 2, 3]);

        for r in &results {
            assert!(!r.plan_text.is_empty(), "plan_text should be non-empty for task {}", r.task_index);
            assert!(!r.phase2_stdout.is_empty(), "phase2_stdout should be non-empty for task {}", r.task_index);
            assert!(r.phase3_outcome.is_none(), "phase3_outcome should be None when stet_path is None");
        }
    }

    #[test]
    fn parallel_block_sequential_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path()); // parallel: false
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: true },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
        ]);

        assert_eq!(plan.segments, vec![Segment::Parallel(vec![1, 2, 3])]);

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();
        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![1, 2, 3], "sequential fallback preserves order");
    }

    #[test]
    fn parallel_block_single_task_sequential() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config_parallel(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: false },
        ]);

        // Single parallel task demoted to Sequential by compute_segments.
        assert_eq!(
            plan.segments,
            vec![Segment::Sequential(1), Segment::Sequential(2), Segment::Sequential(3)]
        );

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();
        let indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(indices, vec![1, 2, 3]);
    }

    #[test]
    fn parallel_block_respects_max_parallel() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config_parallel(dir.path());
        config.max_parallel = 1;
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: true },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
        ]);

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        assert_eq!(results.len(), 3);
        let mut indices: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        indices.sort();
        assert_eq!(indices, vec![1, 2, 3]);
    }

    /// SP-5.3: Phase 3 runs in block task order (segment order), not thread completion order.
    #[test]
    fn parallel_block_phase3_in_segment_order() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config_parallel(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: true },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
        ]);

        assert_eq!(plan.segments, vec![Segment::Parallel(vec![1, 2, 3])]);

        let results = run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        assert_eq!(results.len(), 3);
        let result_order: Vec<u32> = results.iter().map(|r| r.task_index).collect();
        assert_eq!(
            result_order,
            vec![1, 2, 3],
            "parallel block results must be in segment (block task) order, not completion order"
        );
    }

    #[test]
    fn parallel_block_persists_state_for_all() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config_parallel(dir.path());
        let echo = resolve_echo();
        let state_dir = dir.path().join(".peal");
        let mut state = fresh_state();

        let plan = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: true },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
        ]);

        run_scheduled(&echo, &config, &plan, &mut state, &state_dir, None).unwrap();

        let loaded = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist after run");
        assert_eq!(loaded.completed_task_indices, vec![1, 2, 3]);
    }

    #[test]
    fn parallel_block_failure_persists_completed() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join(".peal");
        let echo = resolve_echo();
        let false_path = crate::cursor::resolve_agent_cmd("false").expect("false must exist");

        // Step 1: run task 1 (sequential) successfully.
        let config = test_config_parallel(dir.path());
        let mut state = fresh_state();

        let plan_step1 = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
        ]);
        run_scheduled(&echo, &config, &plan_step1, &mut state, &state_dir, None).unwrap();
        assert!(state.is_task_completed(1));

        // Step 2: run with `false` agent; tasks 2,3 form a parallel block and fail.
        let plan_step2 = make_plan(vec![
            Task { index: 1, content: "A.".to_owned(), parallel: false },
            Task { index: 2, content: "B.".to_owned(), parallel: true },
            Task { index: 3, content: "C.".to_owned(), parallel: true },
        ]);

        let err = run_scheduled(&false_path, &config, &plan_step2, &mut state, &state_dir, None)
            .unwrap_err();

        match err {
            PealError::PhaseNonZeroExit { phase, .. } => assert_eq!(phase, 1),
            other => panic!("expected PhaseNonZeroExit, got: {other:?}"),
        }

        // Task 1 persisted from step 1; tasks 2,3 not completed.
        let loaded = state::load_state(&state_dir)
            .unwrap()
            .expect("state file should exist");
        assert!(loaded.is_task_completed(1), "task 1 should still be completed");
        assert!(!loaded.is_task_completed(2), "task 2 should not be completed");
        assert!(!loaded.is_task_completed(3), "task 3 should not be completed");
    }
}
