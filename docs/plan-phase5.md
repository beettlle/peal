# Phase 5 — Parallel execution

Source: docs/implementation-plan.md. Goal: Honor `(parallel)` in plan; run phases 1 and 2 concurrently inside a block; run phase 3 sequentially per task after all phase 2s in the block.

## Task 1

SP-5.1 — Execution scheduler. Use parsed parallel blocks from SP-1.1. Execution order: sequential segments and parallel blocks. For a parallel block: identify set of task indices.

**SP-5.1 — Done.** Schedule = `ParsedPlan.segments` (from SP-1.1). Execution order = iteration order over segments. Sequential segment = `Segment::Sequential(u32)`; parallel block = `Segment::Parallel(Vec<u32>)` (set of task indices). Single-task parallel run is represented as `Segment::Sequential`. Runner consumes schedule in `run_scheduled` (src/runner.rs).

## Task 2

SP-5.2 — Concurrent phases 1 and 2. For a parallel block: spawn one execution stream per task (e.g. `tokio` tasks or threads). Each stream: Phase 1 → Phase 2 for that task. Wait for all streams to finish before starting phase 3. Use safe concurrency (no shared mutable state).

**SP-5.2 — Done.** Concurrent P1+P2 via `std::thread::scope` in `run_parallel_block`; one thread per task (capped by `max_parallel`); Phase 3 after all joins; no shared mutable state.

## Task 3

SP-5.3 — Sequential Phase 3 for block. After all phase 2s in block complete: run phase 3 (stet + address) sequentially for each task in the block (task order).

## Task 4

SP-5.4 — Config and edge cases. Config: `parallel` (enable/disable), `max_parallel` (cap concurrent Cursor processes). If `parallel` false or `max_parallel == 1`: run all tasks sequentially. Single-task "block": treat as sequential. One task in block fails: persist only completed indices for that block; exit non-zero unless config "continue with remaining tasks".

### Implementation plan (SP-5.4)

**Goal:** Formalize config semantics and edge-case behavior so that parallel vs sequential and failure handling are config-driven and consistent.

**Current state:**

- **Config:** `parallel` (bool) and `max_parallel` (u32) already exist in `PealConfig`; loaded from CLI, env (`PEAL_PARALLEL`, `PEAL_MAX_PARALLEL`), and file; defaults `parallel = false`, `max_parallel = 4`.
- **Runner:** Parallel path is taken when `config.parallel && pending.len() > 1`. Sequential fallback for a `Parallel(indices)` segment when `parallel` is false or only one task is pending. Single-task parallel block is already represented as `Segment::Sequential` by `compute_segments` in `plan.rs`.
- **Failure in block:** `run_parallel_block` returns `(successes, failures)`. Runner persists completed indices (mark + save_state), runs Phase 3 only for successes, then returns `Err` on any failure (fail-fast). No "continue with remaining tasks" option yet.

**Planned changes:**

1. **Treat `max_parallel == 1` as "no parallelism"**
   - In `run_scheduled`, use the parallel path only when `config.parallel && pending.len() > 1 && config.max_parallel > 1`.
   - If `parallel` is false or `max_parallel == 1`, use the existing sequential fallback for every segment (including `Parallel(indices)`).
   - **Files:** `src/runner.rs` (single condition change).

2. **Single-task block (already done)**
   - No code change. `compute_segments` already emits `Segment::Sequential` for a single parallel task; runner already runs one task per segment in that case.

3. **Persist only completed indices on block failure (already done)**
   - Runner already persists successes for the block before running Phase 3 and before returning `Err`. No change.

4. **Add config "continue with remaining tasks"**
   - **New config:** e.g. `continue_with_remaining_tasks: bool` (default `false`). Naming alternative: `on_task_failure: "fail" | "continue"` (align with `on_findings_remaining`). Recommend bool for simplicity.
   - **Semantics:** When a task in a parallel block fails (P1/P2 or P3): if `continue_with_remaining_tasks` is true, do not return `Err`; log the failure, persist completed indices for that block, advance `position` for the failed task(s), and continue to the next segment. When all segments are done, return `Ok(results)` so process exits 0. If false (default), keep current behavior: return `Err` after persisting completed indices → exit non-zero.
   - **Layers:** Add to `PealConfig`, `FileConfig`, `ConfigLayer`; CLI flag (e.g. `--continue-with-remaining-tasks`), env `PEAL_CONTINUE_WITH_REMAINING_TASKS`, file key `continue_with_remaining_tasks`.
   - **Files:** `src/config.rs` (structs, load, merge, validate if any), `src/cli.rs` (RunArgs), `src/runner.rs` (in `Segment::Parallel` branch: on failures, if config set then continue and do not return `Err`; ensure position and results are consistent).

5. **Exit code**
   - Keep current contract: `main` returns `ExitCode::FAILURE` on `run()` `Err`, `ExitCode::SUCCESS` on `Ok`. With "continue with remaining tasks", `run_scheduled` returns `Ok` so exit is 0; failures are visible in logs and can be reflected in `TaskResult` or logging if desired later.

6. **Tests**
   - **max_parallel == 1:** Extend or add test: plan with `Parallel` block, `config.parallel = true`, `config.max_parallel = 1` → segment runs sequentially (e.g. same order as sequential fallback; no concurrent threads for that block).
   - **continue_with_remaining_tasks:** When true and one task in a block fails: state persists only completed indices for that block; runner returns `Ok`; subsequent segments run; process exits 0. When false: existing behavior (return `Err`, exit non-zero).
   - **Sequential fallback:** Already covered by `parallel_block_sequential_fallback` and `parallel_block_respects_max_parallel`; add or adjust for `max_parallel == 1` explicitly.

**Verification**

- Run existing tests (especially `parallel_block_sequential_fallback`, `parallel_block_respects_max_parallel`, `parallel_block_failure_persists_completed`, `scheduled_fail_fast_in_parallel_block_saves_prior`).
- Add/update tests above; run full test suite and confirm no regressions.

## Task 5

SP-5.5 — Resume and parallel blocks. On resume, if next work is a parallel block: run only tasks in block not in `completed_task_indices`; run those in parallel (phases 1+2) then phase 3 sequentially; skip completed tasks in block.

**SP-5.5 — Done.** Behavior implemented in `run_scheduled` (src/runner.rs): `pending` = tasks in block with `!peal_state.is_task_completed(*idx)`; only `pending` is passed to `run_parallel_block`; Phase 3 runs in segment order and skips completed tasks via `successes_by_index.remove(&idx)` → `None` → `continue`. Fully completed block skips with `position += indices.len()`. Sequential fallback uses the same `pending` filter and `position += completed_in_block`. Tests: `scheduled_resume_within_parallel_block` (sequential fallback), `scheduled_resume_parallel_block_concurrent_path` (concurrent path).

### Implementation plan (SP-5.5)

**Goal:** Ensure on resume, when the next work is a parallel block, only incomplete tasks in the block run; they run with P1+P2 in parallel then P3 in segment order; completed tasks in the block are skipped.

**Current state (pre-implementation check):**

- **Concurrent path** (`run_scheduled`, `Segment::Parallel` when `config.parallel && pending.len() > 1 && config.max_parallel > 1`):
  - `pending` is computed as `indices.iter().filter(|idx| !peal_state.is_task_completed(*idx))` → only tasks not in `completed_task_indices` run.
  - `run_parallel_block(..., &pending, ...)` runs P1+P2 only for pending tasks.
  - Phase 3 loop iterates `for idx in indices`; for each, `successes_by_index.remove(&idx)` — already-completed tasks were not in `successes`, so we `continue` and skip P3 for them. Only tasks that ran P1+P2 in this block get P3.
  - Fully-completed block: `pending.is_empty()` → skip block, `position += indices.len()`.
- **Sequential fallback** (same segment when `parallel` false or `pending.len() <= 1` or `max_parallel == 1`):
  - Loop is `for idx in &pending` → only pending tasks run; `position += completed_in_block` at end accounts for skipped tasks.
- **Existing test:** `scheduled_resume_within_parallel_block` (runner.rs) resumes with tasks 1,2 completed; plan has sequential 1, parallel block (2,3), sequential 4; asserts results are [3, 4]. It uses `test_config` (parallel=false), so it only covers the sequential fallback for the block.

**Conclusion:** Behavior for SP-5.5 is already implemented in both paths. Remaining work: verify, add a test that exercises the concurrent path on resume, and mark task done in docs.

**Planned steps:**

1. **Verify implementation**
   - Trace `run_scheduled` for `Segment::Parallel(indices)` with `peal_state` containing some but not all of `indices`:
     - Confirm `pending` excludes completed; confirm Phase 3 loop skips completed (no entry in `successes_by_index`).
   - Confirm sequential fallback uses `pending` only and advances position correctly.

2. **Add test: resume into parallel block (concurrent path)**
   - **Setup:** Plan with one sequential task (1) and one parallel block (2, 3, 4). `config.parallel = true`, `max_parallel > 1`.
   - **Resume state:** Mark task 1 and task 2 completed (so block has one completed, two pending).
   - **Run:** `run_scheduled(...)`.
   - **Assert:** Only tasks 3 and 4 run; results order is segment order (3, 4); state after run has 1, 2, 3, 4 in `completed_task_indices`. Optionally assert that the concurrent path was used (e.g. log or run with two pending so we know `run_parallel_block` was called).
   - **File:** `src/runner.rs` (new test, e.g. `scheduled_resume_parallel_block_concurrent_path`).

3. **Document completion**
   - In `docs/plan-phase5.md`, under Task 5, add a short "SP-5.5 — Done" note: behavior in `run_scheduled` (pending filter, `run_parallel_block` on pending, Phase 3 in segment order skipping completed); reference `scheduled_resume_within_parallel_block` and the new concurrent-path test.

**Verification**

- Run `cargo test scheduled_resume_within_parallel_block` and the new test; run full test suite.
- Manually (optional): run a plan with a parallel block, interrupt after one task in the block completes, resume and confirm only remaining tasks in the block run (P1+P2 parallel, P3 sequential).
