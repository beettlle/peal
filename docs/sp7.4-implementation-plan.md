# SP-7.4 Implementation Plan — Config and docs

**Task (SP-7.4):** Document `normalize_plan`, `--normalize`, when to use (arbitrary input vs. already canonical). Optional: config path for normalization prompt or inline prompt template. State/resume: document that resume uses the plan actually run (file or normalized output); if user re-normalizes, task identity may change.

---

## 1. Summary

- **Required:** Add documentation for normalization (`normalize_plan`, `--normalize`), when to use it, and state/resume behavior.
- **Optional:** Add config support for a custom normalization prompt (file path or inline template).

---

## 2. Current behavior (no code changes for docs-only items)

- **Config:** `normalize_plan` (bool, default `false`) in TOML; `PEAL_NORMALIZE_PLAN` in env; merged with CLI > env > file.
- **CLI:** `--normalize` (bool, default false); `--normalize-retries` (optional u32).
- **Flow:** Plan file is read; if content matches canonical format (`^## Task\s+\d+`), it is parsed directly. If not and normalization is enabled, `plan::normalize_via_agent` runs once (or with retries on parse failure); stdout is parsed as the plan. State/resume keys off `plan_path` and `repo_path` only (`PealState.matches_context`); the “plan actually run” is the parsed plan (from file or from normalized output). The normalized text is not persisted in state.

---

## 3. Documentation (required)

### 3.1 Configuration reference (`docs/configuration.md`)

- **Add to the configuration keys table:**  
  - `normalize_plan` | `normalize_plan` | `NORMALIZE_PLAN` (bool) | `--normalize` | bool | `false`  
  - `normalize_retry_count` | `normalize_retry_count` | `NORMALIZE_RETRY_COUNT` | `--normalize-retries` | u32 | `0`
- **Add a subsection “Plan normalization”** that covers:
  - **What it does:** When the plan file does not match the canonical format (e.g. no `## Task N` headings), and `normalize_plan` or `--normalize` is true, peal invokes the Cursor CLI once with the file content and instructions to convert it to canonical format; the agent’s stdout is then parsed as the plan.
  - **When to use `--normalize` / `normalize_plan = true`:**
    - **Arbitrary input:** PRDs, implementation plans, notes, or other free-form docs you want turned into a PEAL plan in one shot.
  - **When not to use:**
    - **Already canonical:** If the file already has `## Task 1`, `## Task 2`, … headings, peal detects that and parses directly; no agent call. Adding `--normalize` does nothing in that case (no extra invocation).
  - **Precedence:** CLI `--normalize` overrides env and file; same as other options.

### 3.2 State and resume (`docs/configuration.md`)

- **Add or extend a “State and resume” subsection** (e.g. under “Default state path” or as its own section) to state:
  - Resume uses the **plan actually run**: that is, the parsed plan used for that run — either the file content (when canonical) or the **normalized output** from the single normalization invocation. State is keyed by `plan_path` and `repo_path` only; the content (file vs normalized) is not stored in state.
  - **Re-normalizing:** If you run again with the same `--plan` and `--repo` but with normalization enabled (or with a modified source file), the LLM may produce different normalized output. Task identity (Task 1, Task 2, …) and count can change. Resuming will still match on `plan_path` and `repo_path` and skip by **task index**; those indices may no longer correspond to the same logical tasks. So if you re-normalize, treat it as a new run: consider clearing state (e.g. remove `.peal/state.json`) or using a different `state_dir` if you need a clean resume.

### 3.3 Implementation plan / Phase 7 (`docs/implementation-plan.md` or `docs/plan-phase7.md`)

- **Mark SP-7.4 as implemented** in the phase table (e.g. add a short “SP-7.4 implemented” note) and point to the new docs (configuration reference, “Plan normalization”, “State and resume”).

---

## 4. Optional: Custom normalization prompt

**Goal:** Allow a config path to a normalization prompt file, or an inline prompt template, so users can customize the instructions sent to the agent for normalization.

### 4.1 Design choices

- **Option A — Prompt file path:** `normalize_prompt_path: Option<PathBuf>`. If set, read the file; treat its content as the full prompt and replace a single placeholder (e.g. `{{DOC}}` or `---DOC---` … doc … `---DOC---`) with the plan document content. If not set, use the built-in `prompt::normalize_plan_prompt(document_content)`.
- **Option B — Inline template:** `normalize_prompt_template: Option<String>`. Same as above but the template is the string value (e.g. in TOML a multi-line string). Placeholder same as A.
- **Option C — Both:** Support both; precedence: CLI override for path (if we add it) > config path > config template > built-in.

Recommendation: implement **Option A** first (single optional `normalize_prompt_path` in TOML and env, e.g. `PEAL_NORMALIZE_PROMPT_PATH`). Add Option B only if users request it.

### 4.2 Implementation outline (optional)

- **Config:** Add `normalize_prompt_path: Option<PathBuf>` to `PealConfig`, `FileConfig`, `ConfigLayer`; env `PEAL_NORMALIZE_PROMPT_PATH`; merge in `load_file_layer`, `load_env_layer`, `cli_layer_from`, `merge_layers`. No CLI flag in v1 unless needed.
- **Prompt build:** In `plan::normalize_via_agent` (or a small helper in `prompt.rs`): if `config.normalize_prompt_path` is set, read the file, then replace a single designated placeholder (e.g. `{{DOC}}`) with `document_content`; otherwise call `prompt::normalize_plan_prompt(document_content)`. Document the placeholder in configuration.md.
- **Validation:** If `normalize_prompt_path` is set and the file is missing or unreadable, fail at config load or at start of normalization with a clear error.
- **Tests:** Unit test that when `normalize_prompt_path` is set and file contains `{{DOC}}`, the agent receives the file content in place of `{{DOC}}`; when path is None, built-in prompt is used.

### 4.3 Docs (if implemented)

- Add `normalize_prompt_path` to the configuration keys table and a short “Custom normalization prompt” note: path to a file whose content is the full normalization prompt, with one placeholder (e.g. `{{DOC}}`) replaced by the plan document content; if not set, built-in prompt is used.

---

## 5. Files to touch

| Item | File | Action |
|------|------|--------|
| Config keys table | `docs/configuration.md` | Add `normalize_plan`, `normalize_retry_count` |
| Plan normalization | `docs/configuration.md` | New subsection: what it does, when to use, when not to use |
| State/resume | `docs/configuration.md` | New or extended “State and resume” subsection |
| Phase 7 status | `docs/implementation-plan.md` and/or `docs/plan-phase7.md` | Mark SP-7.4 done, reference new docs |
| Optional prompt path | `src/config.rs` | Add `normalize_prompt_path` to config types and merge |
| Optional prompt path | `src/prompt.rs` and/or `src/plan.rs` | Build prompt from file + placeholder or built-in |
| Optional prompt path | `docs/configuration.md` | Document `normalize_prompt_path` and placeholder |

---

## 6. Verification

- **Docs:** Read through `docs/configuration.md` to ensure “Plan normalization” and “State and resume” are clear and accurate.
- **Optional:** If implementing custom prompt path, run existing normalization tests and add a test that uses a file with `{{DOC}}` and assert the agent receives the document content in the prompt.

---

## 7. Order of work

1. **Required (docs):** Update `docs/configuration.md` (keys table, “Plan normalization”, “State and resume”), then update `docs/implementation-plan.md` and/or `docs/plan-phase7.md` to mark SP-7.4 implemented.
2. **Optional:** Implement and document `normalize_prompt_path` (config, prompt build, tests, docs).
