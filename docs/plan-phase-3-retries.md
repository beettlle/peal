# Plan: Phase 3 retries

Add retries for the Phase 3 (address findings) Cursor CLI invocation: retry on timeout or non-zero exit up to a small, capped count; ensure state is saved before returning an error.

---

## 1. Goal

- **Retry Phase 3 Cursor CLI calls** on timeout or non-zero exit, up to a configured count (cap 1–2 retries).
- **Config**: New key `phase_3_retry_count` (or reuse `phase_retry_count` for P3 only — this plan uses a dedicated key so P1/P2 stay unchanged and P3 can be capped independently).
- **State**: Ensure state is saved before returning an error (existing paths already do this; verify no new path bypasses it).

---

## 2. Where Phase 3 is invoked

Phase 3 Cursor CLI invocations happen in two places (both in `src/phase.rs`):

- **`run_phase3`** — address-findings step: builds prompt from stet output and runs the agent. Called from `stet::address_findings` → used inside `address_loop` and `address_loop_custom`.
- **`run_phase3_triage`** — triage step: “Anything to address from this review?”. Called from `stet::dismiss_non_actionable_and_rerun`.

Neither currently retries; they invoke once and return on timeout or non-zero exit. P1 and P2 already use `config.phase_retry_count` with a retry loop in `phase.rs`.

---

## 3. Config (`src/config.rs`)

- **New key**: `phase_3_retry_count: u32`, default `0`. Cap at **2** (so at most 2 retries = 3 total attempts). Document that values &gt; 2 are effectively capped at 2 when used.
- **Layers**: Add to `PealConfig`, `FileConfig`, `ConfigLayer`; merge in `merge_layers`; resolve in `load_with_env` with default `DEFAULT_PHASE_3_RETRY_COUNT` (0). Cap when **using** the value (in `phase.rs`), not in config, so file/env/CLI can set any u32 and behavior stays bounded.
- **Env**: `PEAL_PHASE_3_RETRY_COUNT` (optional u32).
- **CLI**: `--phase-3-retry-count` (optional u32) in `RunArgs`; add to `cli_layer_from`.
- **Constants**: `DEFAULT_PHASE_3_RETRY_COUNT: u32 = 0`. In phase.rs use `effective_retries = config.phase_3_retry_count.min(2)`.

No change to `phase_retry_count` (still used only for P1/P2).

---

## 4. Phase 3 retry logic (`src/phase.rs`)

- **`run_phase3`**  
  - Compute `max_attempts = 1 + config.phase_3_retry_count.min(2)`.  
  - Wrap the single invocation in a `for attempt in 1..=max_attempts` loop (same pattern as `run_phase1` / `run_phase2`).  
  - Use existing `check_result(3, task_index, config.phase_timeout_sec, &result)` for timeout and non-zero exit.  
  - On `Err(e)` from `check_result`: if `attempt < max_attempts`, log warning (“phase 3 failed, retrying”) and continue; else return `Err(e)`.  
  - Log attempt/max_attempts in the existing “invoking phase 3” info line.

- **`run_phase3_triage`**  
  - Same retry loop and cap.  
  - Current behavior: on non-zero exit it returns `Ok(PhaseOutput { stdout: String::new(), … })` (treat as unparseable). For retries: treat timeout and non-zero as retriable. So: run the command; if timeout, retry as above; if non-zero, either retry or keep current “unparseable” behavior. Task says “retry on timeout or non-zero exit”, so retry on both: use a single `check_result`-like check for timeout and, if we want to retry on non-zero for triage, then on non-zero run the same retry logic and only after exhausting retries return the current fallback (empty stdout). So: one loop, max_attempts = 1 + cap(phase_3_retry_count); on timeout or non-zero, retry; after last attempt, triage keeps current behavior for non-zero (return Ok with empty stdout) and returns Err on timeout.

  **Refinement for triage**: `run_phase3_triage` today does not use `check_result`; it handles timeout (return Err) and non-zero (return Ok with empty stdout). For consistency with “retry on timeout or non-zero”:
  - Option A: Retry on both. After retries exhausted: timeout → Err; non-zero → keep current Ok(empty stdout).
  - Option B: Retry only on timeout; non-zero remains “unparseable” without retry.

  Plan adopts **Option A**: retry on timeout and non-zero; after exhausting retries, timeout → Err, non-zero → Ok(empty stdout) as today.

Implement by: introduce a small helper or inline loop that runs the command and then, for triage, maps result to success / retry / fail (timeout → retry then Err; non-zero → retry then Ok empty).

---

## 5. Runner and state save (`src/runner.rs`)

- **No new call sites**: Phase 3 is run via `stet::address_loop` / `stet::address_loop_custom` → `stet::address_findings` → `phase::run_phase3`, and via `stet::dismiss_non_actionable_and_rerun` → `phase::run_phase3_triage`. Retries are entirely inside `run_phase3` and `run_phase3_triage`. No changes to runner call sites for retry logic.
- **State save**: When all Phase 3 retries are exhausted, `run_phase3` (or `run_phase3_triage`) returns `Err`. That propagates through `address_findings` → `address_loop` / `address_loop_custom` → `run_single_task` or the parallel-block closure in `run_scheduled`. Those paths already call `state::save_state(peal_state, state_dir)` before `return Err(e)`. **Action**: Confirm every such path still does a best-effort save before returning; no new early-return that skips save.

---

## 6. CLI (`src/cli.rs`)

- Add to `RunArgs`:
  - `phase_3_retry_count: Option<u32>`
  - Help text: e.g. “Number of retries for Phase 3 (address findings) Cursor CLI on timeout or non-zero exit (default: 0, max 2).”
- In `cli_layer_from`, set `phase_3_retry_count: args.phase_3_retry_count`.

---

## 7. Documentation

- **`docs/configuration.md`**: Add `phase_3_retry_count` to the configuration keys table (TOML, env `PEAL_PHASE_3_RETRY_COUNT`, CLI `--phase-3-retry-count`, type u32, default 0). Note that effective retries are capped at 2.

---

## 8. Tests

- **Config**: In `config.rs` tests, add a test that `phase_3_retry_count` is loaded from TOML/env/CLI and default is 0. Optionally test that a value &gt; 2 is accepted by config and that phase.rs caps at 2 (covered by phase test).
- **Phase**: In `phase.rs` tests:
  - `run_phase3`: With `phase_3_retry_count = 1` and a failing agent (e.g. `false`), expect two attempts then Err (or with a “succeeds on second try” stub, one fail then success).
  - `run_phase3_triage`: Same idea for retry then final timeout or non-zero behavior.
- **Runner**: Optionally add an integration test that when Phase 3 exhausts retries and returns Err, state is saved (e.g. load state after run and assert completed indices or state file present). Can be covered by existing `run_all_phase3_failure_saves_state`-style test if it already runs a scenario that triggers Phase 3 failure.

---

## 9. Implementation order

1. **Config**: Add `phase_3_retry_count` to `PealConfig`, `FileConfig`, `ConfigLayer`; default constant; merge and resolve in `load_with_env`; env in `load_env_layer`; file in `load_file_layer`; CLI in `RunArgs` and `cli_layer_from`.
2. **Phase**: Add retry loop to `run_phase3` using capped `phase_3_retry_count` and `check_result`; add retry loop to `run_phase3_triage` with same cap and consistent “retry on timeout or non-zero, then final timeout → Err / non-zero → Ok(empty)”.
3. **Runner**: Audit all places that call Phase 3 (through `address_loop` / `address_loop_custom` / `dismiss_non_actionable_and_rerun`) and confirm state is saved before returning error.
4. **Docs**: Update `docs/configuration.md`.
5. **Tests**: Config tests for new key; phase tests for P3 retry behavior; confirm or add runner test for state save on Phase 3 failure.

---

## 10. Summary

| Area        | Change |
|------------|--------|
| **config.rs** | New `phase_3_retry_count` (default 0); all layers + env + CLI. |
| **phase.rs**  | Retry loop in `run_phase3` and `run_phase3_triage`; max attempts = 1 + min(phase_3_retry_count, 2). |
| **runner.rs** | No retry logic added; verify state save before error on all Phase 3 failure paths. |
| **cli.rs**    | `--phase-3-retry-count` and `RunArgs.phase_3_retry_count`. |
| **docs**      | Document `phase_3_retry_count` and cap. |
| **tests**     | Config load; phase retry (and triage retry); state save on P3 failure. |
