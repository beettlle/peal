# Plan: Default `on_findings_remaining` — align PRD, code, and docs

## Goal

Ensure the **default** for `on_findings_remaining` is defined in one place and that the PRD (§5), `docs/configuration.md`, and `src/config.rs` agree. Either (A) change the default to `"warn"` and document "warn and continue", or (B) keep `"fail"` and update the PRD to state the implementation default is "fail" with rationale.

---

## Current state

| Location | Current wording / value |
|----------|--------------------------|
| **PRD §5** (line 95) | "Default SHALL be documented (e.g. **warn and continue**)." |
| **`src/config.rs`** | `DEFAULT_ON_FINDINGS_REMAINING = "fail"` |
| **`docs/configuration.md`** | Table default: `"fail"`; full TOML example: `on_findings_remaining = "fail"` |

So: PRD text suggests "warn and continue"; implementation and configuration doc use "fail". The task is to pick one default and make all three agree.

---

## Decision: choose default

Before implementing, decide:

- **Option A — Default `"warn"`:** Matches PRD's "e.g. warn and continue". Better for unattended / "get as much done as possible" runs. Requires code and test changes.
- **Option B — Default `"fail"`:** Keeps current behavior. Strict-by-default: run fails if findings remain after max rounds; user opts in to "warn and continue" via config. PRD is updated to state that the implementation default is "fail" with a short rationale.

Recommendation: If the product goal is strict-by-default (fail fast, explicit opt-in to continue), choose **Option B** and update the PRD. If the product goal is tolerant-by-default (continue and warn), choose **Option A** and change the constant plus tests/docs.

---

## Option A: Change default to `"warn"`

### 1. Code

- **`src/config.rs`**
  - Set `DEFAULT_ON_FINDINGS_REMAINING` to `"warn"`.

### 2. Tests and fixtures

Update every place that assumes the default is `"fail"` when no config is set:

- **`src/config.rs`** (tests):
  - `defaults_applied_when_only_required_fields_present`: expect `on_findings_remaining` `"warn"`.
  - `documented_example_toml_parses_successfully`: TOML in test should match docs (after docs update); assertion for `on_findings_remaining` → `"warn"` when key is omitted, or keep explicit `on_findings_remaining = "warn"` in example and assert `"warn"`.
  - `partial_toml_fills_defaults`: expect `"warn"`.
  - `on_findings_remaining_defaults_to_fail`: rename to `on_findings_remaining_defaults_to_warn` and assert `"warn"`.

Fixtures in **`src/runner.rs`**, **`src/phase.rs`**, **`src/plan.rs`**, **`src/stet.rs`** that construct config with `on_findings_remaining: "fail".to_owned()`:

- Decide per test: if the test is verifying "when default is used", switch to `"warn"` (or use `DEFAULT_ON_FINDINGS_REMAINING` if a shared constant is available). If the test is verifying "when fail is chosen", keep `"fail"` explicitly.

Search for: `on_findings_remaining: "fail"` and update only those that represent "default" behavior; leave explicit `"fail"` where the scenario is "user set fail".

### 3. Documentation

- **`docs/configuration.md`**
  - In the configuration table, set default for `on_findings_remaining` to `"warn"`.
  - In "Full TOML example", use `on_findings_remaining = "warn"` (or omit and state that default is "warn").
  - Optionally add one sentence: "When findings remain after all address rounds, default is **warn and continue** (configurable via `on_findings_remaining`)."

- **PRD §5** (line 95)
  - Replace "Default SHALL be documented (e.g. warn and continue)" with an explicit statement, e.g.: "Default SHALL be **warn and continue**: the orchestrator marks the task as having remaining findings, logs a warning, and continues to the next task. This default is configurable (e.g. `on_findings_remaining`); see configuration docs."
  - Add a cross-reference to the configuration doc as the single place where the default is defined (e.g. "Default value is defined in the orchestrator configuration; see [configuration](configuration.md).").

### 4. Single source of truth

- Treat **`docs/configuration.md`** as the canonical list of defaults. PRD §5 states the *behavior* ("warn and continue") and points to configuration for the default value and key name.

---

## Option B: Keep default `"fail"`, update PRD and docs

### 1. Code

- **`src/config.rs`**: no change; keep `DEFAULT_ON_FINDINGS_REMAINING = "fail"`.

### 2. Tests and fixtures

- No change; they already expect `"fail"` as default.

### 3. Documentation

- **PRD §5** (line 95)
  - Replace "Default SHALL be documented (e.g. warn and continue)" with something like:
    - "When findings remain after max address rounds, the orchestrator SHALL either **fail the task** (default) or **warn and continue** to the next task, as configured (e.g. `on_findings_remaining`). **Implementation default is fail**: strict-by-default so that runs do not silently continue with open findings unless the user configures warn-and-continue. The default value is documented in the configuration reference."
  - Add one sentence: "See [configuration](configuration.md) for the default value and config key."

- **`docs/configuration.md`**
  - Keep table and TOML example as today (default `"fail"`).
  - Optionally add a short note under the `on_findings_remaining` row or in "Edge cases and phase behavior": "Default is **fail** (strict-by-default); set to `warn` for warn-and-continue. PRD §5 describes behavior; this table is the source of truth for the default value."

### 4. Single source of truth

- **`docs/configuration.md`** remains the canonical place for the default value. PRD §5 describes semantics and states that the implementation default is "fail" with rationale, and points to configuration for the exact key and value.

---

## Verification

- **Option A:** Run full test suite; grep for `on_findings_remaining` and `DEFAULT_ON_FINDINGS_REMAINING` and confirm no remaining assertions or comments that say default is "fail". Read PRD §5 and configuration.md and confirm they both say "warn" and "warn and continue".
- **Option B:** Read PRD §5 and configuration.md and confirm PRD says "implementation default is fail" with rationale and references configuration; configuration.md still says default `"fail"`. No code/tests changed.

---

## Summary

| Step | Option A (default `"warn"`) | Option B (default `"fail"`) |
|------|-----------------------------|-----------------------------|
| `DEFAULT_ON_FINDINGS_REMAINING` | Change to `"warn"` | No change |
| Config tests & fixtures | Update expectations and any default-usage fixtures to `"warn"` | No change |
| `docs/configuration.md` | Default → `"warn"`; optional "warn and continue" sentence | Optional note that default is "fail" and PRD §5 references this |
| PRD §5 | State default is "warn and continue"; link to configuration | State implementation default is "fail" with rationale; link to configuration |
| Single source of truth | Configuration doc holds default value; PRD describes behavior | Same |

Do one of the two options end-to-end so that PRD, code, and docs agree and "default" is defined in one place (configuration doc), with PRD §5 either matching it (A) or explicitly documenting the "fail" default and rationale (B).
