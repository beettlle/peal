# Plan: Plan-text sanity check after Phase 1

Optional validation of Phase 1 stdout (plan text) after capture. When enabled, if the plan text fails a configured check (e.g. minimum length), retry P1 once then fail with a clear error.

---

## Goal

- After capturing P1 stdout, optionally validate that plan text meets a configured threshold (minimum length and/or presence of expected tokens).
- If validation fails: retry P1 once (if a retry is allowed), then re-validate; if it still fails, return a clear error (e.g. "Phase 1 returned empty or invalid plan").
- Add config so the feature is off by default (backward compatible).

---

## 1. Config

**New fields on `PealConfig`:**

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `validate_plan_text` | `bool` | `false` | When true, run plan-text validation after each P1 success. |
| `min_plan_text_len` | `Option<usize>` | `None` | When `validate_plan_text` is true: require `plan_text.len() >= min_plan_text_len`. If `None`, require only non-empty (`len() > 0`). |

**Sources (same precedence as existing keys):**

- **TOML:** `validate_plan_text`, `min_plan_text_len`
- **Env:** `PEAL_VALIDATE_PLAN_TEXT` (bool), `PEAL_MIN_PLAN_TEXT_LEN` (u64, stored as `usize`)
- **CLI:** `--validate-plan-text` (bool), `--min-plan-text-len <n>` (optional u64)

**Files to touch:**

- `src/config.rs`: Add to `PealConfig`, `FileConfig`, `ConfigLayer`; merge in `merge_layers`; set in `load_with_env`; add to `load_file_layer`, `load_env_layer`, `cli_layer_from` (and `RunArgs` in `src/cli.rs`).
- `docs/configuration.md`: Document the two new keys in the table and add a short "Plan-text validation" subsection.

---

## 2. Error type

**New variant in `src/error.rs`:**

```rust
#[error("Phase 1 returned empty or invalid plan (task {task_index}): {detail}")]
Phase1PlanTextInvalid { task_index: u32, detail: String },
```

Use for both "empty" and "below minimum length" (and later "missing expected tokens" if added). Example `detail`: `"plan text length 0 (minimum 100)"`.

---

## 3. Validation helper

**Location:** `src/runner.rs` (or a small dedicated module if preferred; keeping it in runner is enough for a single predicate).

**Signature:**

```rust
/// When config.validate_plan_text is true, checks plan text length (and optionally
/// expected tokens). Returns Ok(()) if disabled or valid, Err(PealError::Phase1PlanTextInvalid)
/// if invalid.
fn validate_plan_text(
    config: &PealConfig,
    task_index: u32,
    plan_text: &str,
) -> Result<(), PealError>
```

**Logic:**

- If `!config.validate_plan_text` → return `Ok(())`.
- If `plan_text.is_empty()` → `Err(Phase1PlanTextInvalid { task_index, detail: "plan text is empty".into() })`.
- If `let Some(min) = config.min_plan_text_len` and `plan_text.len() < min` → `Err(Phase1PlanTextInvalid { task_index, detail: format!("plan text length {} below minimum {}", plan_text.len(), min) })`.
- Otherwise → `Ok(())`.

Optional later extension: `config.plan_text_required_tokens: Option<Vec<String>>`; if present, require each token to appear in `plan_text` (e.g. for structure). Not in initial scope.

---

## 4. Retry semantics

- **Single retry for plan-text validation:** On first validation failure, call `phase::run_phase1` again once (no change to `phase.rs`). After the second P1 result, run validation again; if it fails again, return `PealError::Phase1PlanTextInvalid` (do not retry further).
- **Independent of `phase_retry_count`:** Existing retries in `phase::run_phase1` are for process failure (timeout / non-zero exit). Plan-text validation runs only when P1 has already returned success (exit 0). So we have:
  - Process failure → up to `phase_retry_count` retries inside `run_phase1`.
  - Success but invalid plan text → one additional P1 run in the runner, then fail if still invalid.

---

## 5. Call sites (runner)

P1 is run and its stdout used in three places. After every successful `phase::run_phase1` that produces `p1_output`, run validation and optionally retry P1 once.

### 5.1 `run_phase1_all`

- After `phase::run_phase1(...)?` and before `peal_state.mark_task_completed` and `results.push`:
  - Call `validate_plan_text(config, task.index, &output.stdout)`.
  - If `Err(e)`:
    - Log warning (task index, that validation failed, retrying P1 once).
    - Call `phase::run_phase1(...)` again; on that call’s `Err`, return that error (same as today).
    - On success, set `output = second_result` and call `validate_plan_text(config, task.index, &output.stdout)` again; if still `Err`, return that error.
  - Then continue as today (mark completed, save state, push to results).

### 5.2 `run_single_task`

- After the first `phase::run_phase1(...)?` and before the "Phase 2" block:
  - Same pattern: `validate_plan_text(config, task.index, &p1_output.stdout)`; on Err, retry P1 once (reassign `p1_output`), validate again; if still Err, return it (after best-effort state save and error log as elsewhere in this function).
  - Then proceed to Phase 2 with the (possibly updated) `p1_output`.

### 5.3 `run_phases_1_2`

- After `phase::run_phase1(...)?` and before Phase 2:
  - Same pattern: validate; on Err, retry P1 once, validate again; if still Err, return it.
  - Then run Phase 2 with the (possibly updated) plan text.

---

## 6. Tests

- **Config:** Defaults: `validate_plan_text == false`, `min_plan_text_len == None`. Load from TOML/env/CLI and merge correctly.
- **Validation helper:**
  - When `validate_plan_text` is false, always Ok.
  - When true and plan empty → Phase1PlanTextInvalid.
  - When true and `min_plan_text_len == Some(100)` and len 50 → Phase1PlanTextInvalid with detail mentioning 50 and 100.
  - When true and len >= min (or min is None and non-empty) → Ok.
- **Runner integration:**
  - With `validate_plan_text = true`, `min_plan_text_len = Some(1000)`, and an agent that returns short output (e.g. `echo "x"`): after one retry, run should fail with `Phase1PlanTextInvalid`.
  - With validation disabled, existing tests unchanged (backward compatibility).
  - Optional: agent that returns short on first call and long on second (e.g. script) to assert one retry succeeds.

---

## 7. Docs

- **configuration.md:** Add the two keys to the configuration table; add a short "Plan-text validation" subsection describing the feature, that it is off by default, and that one retry is performed on validation failure before failing with Phase1PlanTextInvalid.

---

## 8. Summary checklist

| Item | Action |
|------|--------|
| Config | Add `validate_plan_text`, `min_plan_text_len` to PealConfig, FileConfig, ConfigLayer, RunArgs; wire TOML, env, CLI; defaults false / None. |
| Error | Add `Phase1PlanTextInvalid { task_index, detail }` in error.rs. |
| Helper | Implement `validate_plan_text(config, task_index, plan_text)` in runner (or small module). |
| run_phase1_all | After P1 success, validate; on failure retry P1 once, re-validate; if still fail, return error. |
| run_single_task | Same after first P1 success. |
| run_phases_1_2 | Same after P1 success. |
| Tests | Config defaults and load; validation helper cases; one runner test with validation enabled and short plan (fail after retry). |
| Docs | configuration.md: table + "Plan-text validation" subsection. |

---

## 9. Optional follow-up

- **Expected tokens / structure:** Add `plan_text_required_tokens: Option<Vec<String>>` (or similar); in `validate_plan_text`, require each string to be present in `plan_text` when set. Env could be comma-separated; TOML a string array.
- **Stricter retry cap:** If we ever want to align with `phase_retry_count` (e.g. "validation failure consumes one process retry"), we could pass remaining retries into the runner and decrement on validation retry; current plan keeps validation retry as a single extra attempt for simplicity.
