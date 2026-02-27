# Plan: Run summary and exit semantics

## Goal

- Record a run summary when the process exits successfully (exit 0 or 2).
- Support distinct exit codes: **0** = all clean, **1** = hard failure, **2** = completed with failures or remaining findings.
- Write the summary under `state_dir` by default, with an optional configurable path.
- Document behavior and new config in `docs/configuration.md`.

---

## 1. Exit code semantics

| Exit code | Meaning |
|-----------|--------|
| **0**     | All planned tasks completed; no task failures; no tasks with remaining findings (phase 3 resolved or N/A). |
| **1**     | Hard failure: config/plan error, phase failure (or phase 3 findings-remaining when `on_findings_remaining = "fail"`), stet start/run failure, etc. No summary is written. |
| **2**     | Run completed without hard failure but with **issues**: at least one task failed (e.g. with `continue_with_remaining_tasks`) **or** at least one task has remaining findings (phase 3 ran and `findings_resolved == false`). Summary is written. |

**When to write the summary:** Whenever the run returns `Ok(...)` from the Run command (i.e. before returning to `main`), so for both exit 0 and exit 2. Do **not** write on `Err` (exit 1).

---

## 2. Run summary contents

The summary is a small report with:

- **tasks_completed** — Task indices that completed without failure and without remaining findings (phase 3 absent or `findings_resolved == true`).
- **tasks_failed** — Task indices that were attempted but failed (phase 1, 2, or 3 failure). Only non-empty when `continue_with_remaining_tasks` is true and the runner continued after a failure.
- **tasks_with_remaining_findings** — Task indices that completed phase 2 and phase 3 ran but `findings_resolved == false`.

Optional fields (recommended for usability and scripting):

- **exit_code** — 0 or 2 (the code used for this run).
- **plan_path**, **repo_path** — Copy from config/state for context.
- **timestamp** or **completed_at** — Optional ISO8601 string for “last run” reporting.

Format: JSON under `state_dir` (e.g. `run_summary.json`) or a configurable path. Keep the schema minimal and stable.

---

## 3. Runner changes (`src/runner.rs`)

### 3.1 Return type: optional `RunOutcome` with failed indices

- Today: `run_scheduled` returns `Result<Vec<TaskResult>, PealError>`.
- Change: Introduce a small struct used only when we need to expose “completed with issues” and failed indices:

  **Option A (minimal):** Keep `run_scheduled` returning `Result<Vec<TaskResult>, PealError>`. In `main`, derive summary from `results` only:
  - **tasks_completed**: indices in `results` where `phase3_outcome` is `None` or `Some(o)` with `o.findings_resolved == true`.
  - **tasks_with_remaining_findings**: indices in `results` where `phase3_outcome` is `Some(o)` and `!o.findings_resolved`.
  - **tasks_failed**: leave empty (only non-empty when `continue_with_remaining_tasks` is true and runner continues after a failure; see Option B).

  **Option B (complete):** Extend the runner to return a run-outcome type that includes failed task indices when `continue_with_remaining_tasks` is true:
  - Add something like `RunOutcome { results: Vec<TaskResult>, failed_task_indices: Vec<u32> }`.
  - In `run_scheduled` (and any parallel/sequential paths that skip tasks on failure), collect task indices that failed (e.g. P1/P2 failure in a parallel block, or phase 3 failure when we continue) and return them in `failed_task_indices`.
  - `run_scheduled` returns `Result<RunOutcome, PealError>`; `run_all` and callers adapt to `RunOutcome`.

Recommendation: **Option B** so the report can include “tasks failed” when running with `continue_with_remaining_tasks`. If time is limited, implement Option A first and add Option B in a follow-up.

### 3.2 No change to when state is saved

State is already saved during the run (per task or on failure). No need to change persist logic for resume; the run summary is a separate artifact written only on successful run completion in `main`.

---

## 4. Main changes (`src/main.rs`)

### 4.1 Run command flow

- After `run_result?` (and post-run commands and existing logging), compute:
  - From `results` (and if present, `failed_task_indices`): `tasks_completed`, `tasks_failed`, `tasks_with_remaining_findings` as above.
  - **has_issues** = `!failed_task_indices.is_empty() || results.iter().any(|r| r.phase3_outcome.as_ref().map_or(false, |o| !o.findings_resolved))`.
- Choose exit code for this run:
  - If we already returned `Err` from `run()` → **1** (unchanged).
  - If `run()` returned `Ok(())` and `has_issues` → **2**.
  - If `run()` returned `Ok(())` and `!has_issues` → **0**.
- **Return type of `run()`:** Either:
  - Keep `run(cli) -> Result<(), PealError>` and have `run()` return something like `Result<RunSuccess, PealError>` where `RunSuccess` carries `has_issues` and the data needed to build the summary (e.g. `results` + `failed_task_indices`), or
  - Keep `run()` returning `Result<(), PealError>` and pass a shared struct or callback to record summary data before returning.

Simplest approach: have the Run branch produce a value that encodes “success + summary data” (e.g. `Ok(RunSuccess { results, failed_task_indices })`) and return that from `run()`; then in `main()` map `Ok(success)` to exit 0 or 2 using `has_issues`, and write the summary before exiting.

### 4.2 Writing the summary

- When the Run command succeeds (we have `results` and optional `failed_task_indices`), build the summary struct (tasks_completed, tasks_failed, tasks_with_remaining_findings, plus optional exit_code, plan_path, repo_path, timestamp).
- Resolve path: if config has `run_summary_path: Some(p)` use it; else use `state_dir.join("run_summary.json")`. Create parent dirs if needed; write atomically (e.g. write to temp file then rename) where possible.
- Write only when exit will be 0 or 2; do not write on exit 1.
- On write failure: log warning and do not change exit code (best-effort report).

### 4.3 Exit code in `main()`

- `main()` currently: `run(cli)` → `Ok(())` => `ExitCode::SUCCESS`, `Err(_)` => `ExitCode::FAILURE`.
- Change to:
  - `Err(_)` => `ExitCode::from(1)`.
  - `Ok(RunSuccess { ... })` => compute `has_issues`; if true `ExitCode::from(2)` else `ExitCode::SUCCESS` (0).
  - For non-Run commands (e.g. `Prompt`), keep current behavior (success/failure only; no summary, no exit 2).

---

## 5. Configuration

- **New optional setting:** `run_summary_path` (or `summary_path`).
  - **TOML:** `run_summary_path = "path/to/summary.json"` (optional).
  - **Env:** `PEAL_RUN_SUMMARY_PATH` (optional).
  - **CLI:** `--run-summary-path` (optional).
  - **Default:** None = write to `{state_dir}/run_summary.json`. If set, write to that path (can be absolute or relative to cwd).
- Add to `PealConfig` in `src/config.rs` and merge in the usual precedence (CLI > env > file > default).
- Document in `docs/configuration.md`: exit code table, summary contents, and `run_summary_path` in the configuration keys table and in “State and resume” or a new “Run summary” subsection.

---

## 6. Documentation (`docs/configuration.md`)

- **Exit codes:** Add a subsection (e.g. “Exit codes”) with the table: 0 = all clean, 1 = hard failure, 2 = completed with issues. Mention that 2 is useful for CI/scripts to distinguish “all clean” from “done but with failures or remaining findings”.
- **Run summary:** Add a subsection “Run summary”:
  - When it is written (on successful run, exit 0 or 2).
  - Where it is written (default `{state_dir}/run_summary.json`, or `run_summary_path` if set).
  - What it contains (tasks_completed, tasks_failed, tasks_with_remaining_findings; optional fields as implemented).
- **Configuration keys:** Add a row for `run_summary_path` (TOML key, env var, CLI flag, type, default).
- Optionally mention in “Tolerant vs strict profiles” that exit 2 can occur in tolerant runs when findings remain or tasks are skipped.

---

## 7. Implementation order

1. **Config:** Add `run_summary_path` to `PealConfig` and merge (file, env, CLI). Default `None`.
2. **Runner (Option B):** Introduce `RunOutcome { results, failed_task_indices }` and have `run_scheduled` (and `run_all`) return `Result<RunOutcome, PealError>`. In parallel/continue paths, collect failed task indices into `RunOutcome.failed_task_indices`. Update all call sites and tests that use `run_scheduled`/`run_all` to use `.results` and, where needed, `.failed_task_indices`.
3. **Summary type and path:** Define a `RunSummary` struct (e.g. in `state.rs` or a small new module) with the fields above; add a function to build it from `RunOutcome` + config (plan_path, repo_path) and to write it to a path (create dirs, atomic write, best-effort).
4. **Main:** In the Run branch, use `RunOutcome`; after post_run_commands and logging, compute `has_issues`, build `RunSummary`, write it when exit will be 0 or 2; return from `run()` a type that carries “success + has_issues” (or the summary data); in `main()`, map to exit 0, 1, or 2 and call the summary writer before returning the exit code.
5. **Tests:** Add tests for: exit 0 vs 2 when no remaining findings vs remaining findings; summary file presence and contents when run succeeds; no summary file on failure; optional test for `run_summary_path` override.
6. **Docs:** Update `docs/configuration.md` as in section 6.

---

## 8. Edge cases

- **Prompt command:** No summary, no exit 2; success/failure only.
- **Run fails before any task (e.g. plan parse error):** Exit 1, no summary.
- **Run fails during first task (no continue_with_remaining_tasks):** Exit 1, no summary; state may have been saved for partial progress.
- **Run succeeds with all tasks clean:** Exit 0, summary written with tasks_completed populated, others empty.
- **Run succeeds with one task having remaining findings:** Exit 2, summary written with tasks_with_remaining_findings populated.
- **Run succeeds with continue_with_remaining_tasks and one task failed:** Exit 2, summary written with tasks_failed populated (when Option B is implemented).
- **Summary write fails:** Log warning, still exit 0 or 2 as determined by run outcome.

---

## 9. Files to touch

| File | Changes |
|------|--------|
| `src/main.rs` | Run branch: consume `RunOutcome`, compute has_issues, build and write summary, return success type; map to exit 0/1/2. |
| `src/runner.rs` | Return `RunOutcome` from `run_scheduled`/`run_all`; collect failed_task_indices when continue_with_remaining_tasks. |
| `src/config.rs` | Add `run_summary_path: Option<PathBuf>`, merge from file/env/CLI. |
| `src/state.rs` (or new module) | `RunSummary` struct, build from RunOutcome + config, write to path (atomic, best-effort). |
| `src/cli.rs` | Add `--run-summary-path` if using CLI. |
| `docs/configuration.md` | Exit codes subsection, Run summary subsection, `run_summary_path` in keys table. |

Optional: add a small `run_summary` module (e.g. `src/run_summary.rs`) for the summary struct and write logic to keep `state.rs` focused on resume state.
