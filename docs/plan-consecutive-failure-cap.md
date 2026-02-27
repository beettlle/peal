# Plan: Consecutive-failure cap (`max_consecutive_task_failures`)

## Goal

Add a config option `max_consecutive_task_failures: Option<u32>`. When set, the runner tracks consecutive task failures (across sequential segments and within/between parallel blocks). When the count reaches the cap, the run stops, state is persisted, and the process exits with a **distinct** error so automation can detect "run stopped due to consecutive failures."

---

## 1. Config (`src/config.rs`)

- **PealConfig:** Add field  
  `max_consecutive_task_failures: Option<u32>`  
  No default (None = feature off).

- **FileConfig:** Add  
  `max_consecutive_task_failures: Option<u32>`  
  (optional TOML key).

- **ConfigLayer:** Add  
  `max_consecutive_task_failures: Option<u32>`.

- **Load pipeline:**  
  - In `load_file_layer`: map from `FileConfig`.  
  - In `load_env_layer`: add `parse_env_u32(env_fn, "MAX_CONSECUTIVE_TASK_FAILURES")?`.  
  - In `cli_layer_from`: map from `RunArgs.max_consecutive_task_failures`.  
  - In `merge_layers`: merge with `cli.or(env).or(file)`.  
  - In `load_with_env` (final `Ok(PealConfig { ... })`): assign  
    `max_consecutive_task_failures: merged.max_consecutive_task_failures`.

- **Validation:** No extra validation (optional u32; if present, runner uses it).

- **Tests:** Add at least one test that loads the key from TOML, env, and CLI and that precedence is respected (e.g. `max_consecutive_task_failures_from_toml`, `_from_env`, `_from_cli`, and a precedence test). Update `minimal_cli_args` and any `RunArgs` literals in config tests to include `max_consecutive_task_failures: None` (or the new field) so the struct compiles.

---

## 2. CLI (`src/cli.rs`)

- **RunArgs:** Add  
  `#[arg(long)]`  
  `pub max_consecutive_task_failures: Option<u32>`  
  (e.g. `--max-consecutive-task-failures N`).

- **Tests:** Add a test that parses the flag (e.g. `--max-consecutive-task-failures 3` → `Some(3)`).

---

## 3. Error (`src/error.rs`)

- Add a new variant for automation detection:  
  `ConsecutiveTaskFailuresCapReached { count: u32, cap: u32 }`  
  with a clear `#[error(...)]` message, e.g.  
  `"Run stopped: {count} consecutive task failure(s) reached cap {cap}."`

- This allows automation to match on error type or message (and optionally use a dedicated exit code from `main`).

---

## 4. Runner (`src/runner.rs`)

**Semantics:**

- **Consecutive counter:** One run-wide counter `consecutive_failures: u32`, initially 0.
- **Success:** Any task that completes successfully (P1→P2→P3 or P1→P2 in parallel path and then P3 success) resets the counter to 0.
- **Failure:** Any task that fails (phase failure, stet/address failure, etc.) increments the counter by 1. After incrementing, if `config.max_consecutive_task_failures == Some(cap)` and `consecutive_failures >= cap`, then: best-effort `state::save_state(peal_state, state_dir)`, then return `Err(PealError::ConsecutiveTaskFailuresCapReached { count: consecutive_failures, cap })`.
- **Skip (already completed):** Skipping an already-completed task does **not** change the counter (no success/failure event).
- **Parallel block:** Process outcomes in **segment order** (indices order). For each task in the block: if success → set consecutive to 0; if failure → increment, then check cap and possibly return. So the order of application is the order of `indices` in the segment, with each task’s success/failure applied in that order.

**Places to update:**

1. **`run_scheduled`**
   - Add at the start (e.g. after `failed_task_indices`):  
     `let mut consecutive_failures: u32 = 0;`
   - Pass `consecutive_failures` and `config.max_consecutive_task_failures` into the logic that handles each segment (or a helper that updates counter and checks cap).

2. **Sequential branch (`Segment::Sequential(idx)`)**  
   - After `run_single_task(...)`:
     - If `Ok(result)`: set `consecutive_failures = 0`, push result, continue.
     - If `Err(e)`:  
       `consecutive_failures += 1`.  
       If `config.max_consecutive_task_failures == Some(cap) && consecutive_failures >= cap`:  
       best-effort `state::save_state(...)`, then  
       `return Err(PealError::ConsecutiveTaskFailuresCapReached { count: consecutive_failures, cap })`.  
       Otherwise, existing behavior: save state, return `Err(e)`.

3. **Parallel branch (`Segment::Parallel(indices)`)**  
   - After P1+P2 and Phase 3 handling, we have for each index in the block either success (with `TaskResult`) or failure (in `failed_task_indices` or an early return).
   - When iterating in segment order to apply outcomes:
     - For each task in `indices` in order: if it succeeded, set `consecutive_failures = 0`; if it failed, `consecutive_failures += 1`, then if cap is set and `consecutive_failures >= cap`, save state and return `Err(PealError::ConsecutiveTaskFailuresCapReached { count, cap })`.
   - This must happen at a point where we know success vs failure for every task in the block. Today, when there are P1/P2 failures we break out and return one of those errors; when there are P3 failures we either continue (continue_with_remaining_tasks) or return. So:
     - When returning early due to P1/P2 or P3 failure, before returning: update consecutive counter for the segment (e.g. count failures in order, then successes), then check cap and possibly return `ConsecutiveTaskFailuresCapReached` instead of the inner error (or after updating counter, if cap hit, return cap-reached error; else return original error).
     - When the block completes (all successes or continue_with_remaining_tasks and we’re not returning early): apply outcomes in segment order (success → 0, failure → +1, check cap) and if cap reached, save state and return the new error.
   - Ensure that whenever we would currently `return Err(e)` from inside the parallel block, we first update `consecutive_failures` for the tasks already known to have failed/succeeded in this block (in segment order), then if cap is reached return `ConsecutiveTaskFailuresCapReached`, else persist state and return `Err(e)` as today.

**Helper (optional but recommended):**

- Add a small helper, e.g.  
  `fn check_consecutive_cap(consecutive_failures: u32, cap: u32, peal_state: &mut PealState, state_dir: &Path) -> Result<(), PealError>`  
  that if `consecutive_failures >= cap` does best-effort save and returns  
  `Err(PealError::ConsecutiveTaskFailuresCapReached { count: consecutive_failures, cap })`,  
  otherwise `Ok(())`.  
  Call it after every increment of `consecutive_failures` where we might return.

**Tests:**

- Add a test: sequential run with a stub that fails N times; `max_consecutive_task_failures = Some(N)`; expect `ConsecutiveTaskFailuresCapReached`, state persisted, and (if main uses a distinct exit code) exit code 3.
- Optionally: parallel block with mixed success/failure and cap such that after the block we hit the cap; expect same error and state persisted.

---

## 5. Main (`src/main.rs`)

- In the `Err(e)` arm of `run()`:  
  If `e.downcast_ref::<PealError>()` (or equivalent) is `Some(PealError::ConsecutiveTaskFailuresCapReached { .. })`, then exit with a **distinct exit code** (e.g. **3**) so automation can detect "run stopped due to consecutive failures" without parsing stderr.  
  Other errors remain exit code 1.

- Optional: When exiting with the consecutive-failures error, optionally write a run summary (with exit_code 3) so automation can also rely on `run_summary.json` (e.g. `exit_code: 3`). This requires loading state/outcome to build the summary; if omitted, automation can rely on exit code 3 only.

- **Exit code table (docs):** Document exit code 3 in `docs/configuration.md` (see below).

---

## 6. Documentation (`docs/configuration.md`)

- **Configuration keys table:** Add a row for `max_consecutive_task_failures`:
  - TOML: `max_consecutive_task_failures`
  - Env: `PEAL_MAX_CONSECUTIVE_TASK_FAILURES`
  - CLI: `--max-consecutive-task-failures`
  - Type: u32 (optional)
  - Default: — (not set = no cap)

- **Exit codes section:** Add a row:
  - **3** — Run stopped because the number of consecutive task failures reached `max_consecutive_task_failures`. State was persisted; automation can detect this condition by exit code 3.

- **Edge cases / phase behavior (or new subsection):** Add a short note:
  - When `max_consecutive_task_failures` is set, the runner counts consecutive task failures (success resets the count). When the count reaches the cap, the run stops, state is saved, and the process exits with the distinct error and exit code 3. Applicable in both sequential and parallel execution; in parallel blocks, outcomes are applied in segment (task) order for the purpose of the consecutive counter.

---

## 7. Run summary (optional)

- If main writes a summary on consecutive-failures exit: extend `RunSummary` or the write path so that when we exit with code 3, the summary includes `exit_code: 3` and optionally a field like `stopped_reason: "consecutive_task_failures_cap"` for clarity. Not strictly required if automation can rely on exit code 3 alone.

---

## 8. Implementation order

1. **error.rs** — Add `ConsecutiveTaskFailuresCapReached`.
2. **config.rs** — Add field to `PealConfig`, `FileConfig`, `ConfigLayer`; file/env/CLI load and merge; tests (including `minimal_cli_args` and any full-struct literals).
3. **cli.rs** — Add `--max-consecutive-task-failures` and parse test.
4. **runner.rs** — Add `consecutive_failures` and cap logic in `run_scheduled` for sequential and parallel segments; helper for “check cap and maybe return”; tests.
5. **main.rs** — Map `ConsecutiveTaskFailuresCapReached` to exit code 3; optionally write summary on this path.
6. **docs/configuration.md** — New key, exit code 3, and behavior note.

---

## 9. Summary

| File | Changes |
|------|--------|
| `src/error.rs` | New variant `ConsecutiveTaskFailuresCapReached { count, cap }`. |
| `src/config.rs` | `max_consecutive_task_failures` on PealConfig, FileConfig, ConfigLayer; load from file/env/CLI; merge; tests. |
| `src/cli.rs` | `--max-consecutive-task-failures N` on RunArgs; test. |
| `src/runner.rs` | Track `consecutive_failures`; on each failure increment and optionally return after persist; apply outcomes in segment order in parallel block; tests. |
| `src/main.rs` | On `ConsecutiveTaskFailuresCapReached`, exit with code 3; optionally write summary. |
| `docs/configuration.md` | New config key, exit code 3, and behavior description. |

Automation can detect "run stopped due to consecutive failures" by exit code **3** and/or by matching the error message or type.
