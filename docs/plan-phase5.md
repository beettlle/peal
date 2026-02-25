# Phase 5 — Parallel execution

Source: docs/implementation-plan.md. Goal: Honor `(parallel)` in plan; run phases 1 and 2 concurrently inside a block; run phase 3 sequentially per task after all phase 2s in the block.

## Task 1

SP-5.1 — Execution scheduler. Use parsed parallel blocks from SP-1.1. Execution order: sequential segments and parallel blocks. For a parallel block: identify set of task indices.

## Task 2

SP-5.2 — Concurrent phases 1 and 2. For a parallel block: spawn one execution stream per task (e.g. `tokio` tasks or threads). Each stream: Phase 1 → Phase 2 for that task. Wait for all streams to finish before starting phase 3. Use safe concurrency (no shared mutable state).

## Task 3

SP-5.3 — Sequential Phase 3 for block. After all phase 2s in block complete: run phase 3 (stet + address) sequentially for each task in the block (task order).

## Task 4

SP-5.4 — Config and edge cases. Config: `parallel` (enable/disable), `max_parallel` (cap concurrent Cursor processes). If `parallel` false or `max_parallel == 1`: run all tasks sequentially. Single-task "block": treat as sequential. One task in block fails: persist only completed indices for that block; exit non-zero unless config "continue with remaining tasks".

## Task 5

SP-5.5 — Resume and parallel blocks. On resume, if next work is a parallel block: run only tasks in block not in `completed_task_indices`; run those in parallel (phases 1+2) then phase 3 sequentially; skip completed tasks in block.
