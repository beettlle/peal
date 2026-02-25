# Phase 3 — State and resume

## Task 1

State schema (SP-3.1): Define the state struct with `plan_path`, `repo_path`, `completed_task_indices: Vec<u32>`. Optional fields: `last_plan_by_task`, `last_completed_ref`. Serialize to JSON. Default state file `.peal/state.json`. Config: `state_dir` or path relative to plan/repo; document default `.peal/state.json`.

Full spec: docs/implementation-plan.md Phase 3 SP-3.1.

## Task 2

State read/write (SP-3.2): Load state from configurable path; if missing or invalid, treat as no resume (warn to stderr). After each successful task, append task index to `completed_task_indices` and write state. On failure: persist current state, exit non-zero.

Full spec: docs/implementation-plan.md Phase 3 SP-3.2.

## Task 3

Resume semantics (SP-3.3): On `run` with existing state (same plan path + repo path): load state, skip tasks in `completed_task_indices`, run from smallest index not in set. If next work is a parallel block: run only tasks in that block not in set (parallel block behavior in Phase 5).

Full spec: docs/implementation-plan.md Phase 3 SP-3.3.

## Task 4

Single task and range (SP-3.4): CLI: `--task N` (run only task N), `--from-task N` (run from task N to end). State may be updated so resume is consistent.

Full spec: docs/implementation-plan.md Phase 3 SP-3.4.
