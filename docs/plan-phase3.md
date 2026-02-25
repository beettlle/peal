# Phase 3 — State and resume

This phase adds state persistence and **resume from failed or canceled runs** (skip completed tasks, run from first incomplete).

**Instructions for the agent:** Before implementing each task, search the codebase to see if the requested behavior already exists (e.g. state struct, load/save state, runner skipping completed tasks, CLI `--task` / `--from-task`). If it already exists: tell the user that the task is already implemented, cite the relevant code, and proceed to the next task (if any). If it does not exist: implement the task as specified.

## Task 1

State schema (SP-3.1): Define the state struct with `plan_path`, `repo_path`, `completed_task_indices: Vec<u32>`. Optional fields: `last_plan_by_task`, `last_completed_ref`. Serialize to JSON. Default state file `.peal/state.json`. Config: `state_dir` or path relative to plan/repo; document default `.peal/state.json`.

Full spec: docs/implementation-plan.md Phase 3 SP-3.1.

## Task 2

State read/write (SP-3.2): Load state from configurable path; if missing or invalid, treat as no resume (warn to stderr). After each successful task, append task index to `completed_task_indices` and write state. On failure: persist current state, exit non-zero.

Full spec: docs/implementation-plan.md Phase 3 SP-3.2.

## Task 3

**Goal:** Allow users to resume from a failed or canceled run: re-run with the same plan and repo; peal skips tasks in `completed_task_indices` and continues from the first incomplete task.

Resume semantics (SP-3.3): On `run` with existing state (same plan path + repo path): load state, skip tasks in `completed_task_indices`, run from smallest index not in set. If next work is a parallel block: run only tasks in that block not in set (parallel block behavior in Phase 5).

Full spec: docs/implementation-plan.md Phase 3 SP-3.3.

## Task 4

Single task and range (SP-3.4): CLI: `--task N` (run only task N), `--from-task N` (run from task N to end). State may be updated so resume is consistent.

Full spec: docs/implementation-plan.md Phase 3 SP-3.4.
