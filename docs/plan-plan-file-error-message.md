# Plan: Plan file error message (PRD §4 / error table)

**Goal:** Either make the user-facing message for invalid/missing plan file exactly match the PRD, or relax the PRD to require only a "clear error."

---

## 1. Current state

### PRD

- **Encoding (PRD §4, ~line 74):** "Invalid UTF-8 SHALL cause \"Invalid or missing plan file\" before any task runs." (no period)
- **Edge cases table (PRD ~line 280):** "Exit before starting any task; message \"Invalid or missing plan file.**\"**" (with period)

So the PRD specifies the exact phrase; the table adds a trailing period.

### Code

- **`src/error.rs`:** Both `InvalidPlanFile` and `PlanFileNotFound` use:
  - `#[error("Invalid or missing plan file: {path}")]`
  - User sees: `Invalid or missing plan file: /path/to/file` (path appended).

### Display path

- Errors are constructed in: `src/plan.rs` (`parse_plan_file`), `src/config.rs` (`validate()`), `src/main.rs` (map_err when reading plan content and when normalization produces no tasks).
- They are shown to the user only via `Display` from `thiserror::Error`: in `main.rs`, `error!("{e:#}");` prints the error chain. No custom formatting elsewhere for these variants.
- **Single source of truth for user-facing text:** the `#[error("...")]` strings in `src/error.rs`.

### Tests

- `src/main.rs`: `err_msg.contains("Invalid or missing plan file")` (substring).
- `src/config.rs`: same.
- `src/plan.rs`: `msg.contains("Invalid or missing plan file") || msg.contains("invalid utf-8")` and `msg.contains("does not exist") || msg.contains("Invalid or missing plan file")`.

Tests do not assert the exact message; they allow extra text (e.g. path).

---

## 2. Options

### Option A — Match PRD exactly

- **Change `src/error.rs`:**
  - Set both `InvalidPlanFile` and `PlanFileNotFound` to a fixed message: either `"Invalid or missing plan file"` or `"Invalid or missing plan file."` (pick one and use it consistently; recommend the table’s version with period).
  - Do **not** include `{path}` in the user-facing message (PRD does not require it).
- **Optional:** If we want to keep path for logs/debugging, we can:
  - Add a separate method or `Debug` that includes the path, or
  - Log the path at the call site when returning the error (e.g. in `main` or in `plan.rs`/`config.rs` before returning). PRD only constrains the **user-facing** message.
- **Tests:** Update any test that relies on the path in the message (e.g. `config.rs` “does not exist” / “Invalid or missing plan file”) to assert the exact message or the chosen substring, and fix `plan.rs` tests that accept “does not exist” so they still pass (message will no longer include path; “Invalid or missing plan file” will still be present).

### Option B — Relax the PRD

- **Change PRD only:**
  - In §4 (Encoding): replace "Invalid UTF-8 SHALL cause \"Invalid or missing plan file\"" with wording that the orchestrator SHALL exit with a **clear error** before any task runs (no exact phrase).
  - In the edge cases table (Plan file missing or unparseable): replace the exact message with "clear error" (e.g. "Exit before starting any task with a clear error.").
- **Code:** No change. Current message "Invalid or missing plan file: {path}" is already clear.
- **Tests:** No change (they already use substring).

---

## 3. Recommendation

- **Prefer Option A** if you want PRD and implementation to be strictly aligned and consistent with the rest of the error table (other rows use exact phrases like "Target path is not a git repository").
- **Prefer Option B** if you want to keep the path in the message for support/debugging and are fine with the PRD only requiring a clear error.

---

## 4. Implementation steps (Option A)

1. **Decide exact string:** Use `"Invalid or missing plan file."` (with period) to match the error table; document in a short comment in `src/error.rs` that this string is required by PRD.
2. **Edit `src/error.rs`:**
   - For `InvalidPlanFile` and `PlanFileNotFound`, set `#[error("Invalid or missing plan file.")]` (no `{path}` in the message).
   - Keep the struct fields (`path: PathBuf`) so call sites and logging can still use the path; optionally implement or use `Display`/`Debug` to expose path only in logs or debug output if desired.
3. **Call sites:** No change required; they already construct the variants with `path`. No code currently parses the error message for the path.
4. **Tests:**
   - **main.rs:** Update assertions to require the exact message (e.g. `assert_eq!(err_msg.trim(), "Invalid or missing plan file.")`) or keep `contains("Invalid or missing plan file")` and add a note that the message must not include extra detail if we later add any.
   - **config.rs:** Same: either exact match or `contains`; fix the test that expects "does not exist" — after change, both missing file and invalid file will show the same PRD message, so assert on "Invalid or missing plan file" only.
   - **plan.rs:** `parse_plan_file_not_found` currently expects "does not exist" or "Invalid or missing plan file"; after change, message will be only "Invalid or missing plan file." so assert for that. `parse_plan_file_invalid_utf8` already accepts "Invalid or missing plan file" — tighten to exact message if desired.
5. **PRD:** If we ever want to allow the path in the message later, add a note in the PRD that the implementation may append the path; otherwise leave PRD as-is.

---

## 5. Implementation steps (Option B)

1. **Edit `docs/cursor-orchestrator-prd.md`:**
   - §4 Encoding bullet: replace the exact phrase with: invalid UTF-8 SHALL cause a **clear error** and exit before any task runs.
   - Edge cases table, "Plan file missing or unparseable": replace message "Invalid or missing plan file." with "clear error" (e.g. "Exit before starting any task with a clear error.").
2. **Code and tests:** No changes.

---

## 6. Files to touch

| Option | Files |
|--------|--------|
| A      | `src/error.rs` (message string; optionally logging of path); `src/main.rs` (tests); `src/config.rs` (tests); `src/plan.rs` (tests) |
| B      | `docs/cursor-orchestrator-prd.md` only |

---

## 7. Verification

- **Option A:** Run `cargo test`; run with a missing or invalid plan file and confirm stderr shows exactly "Invalid or missing plan file." (or chosen variant).
- **Option B:** Read PRD §4 and the edge cases table and confirm they require only a "clear error"; run once with missing plan and confirm message is still clear and includes path.
