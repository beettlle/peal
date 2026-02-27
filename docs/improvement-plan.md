# PEAL Improvement Plan

Source: consolidated PRD audit (Opinions 1–4). This plan addresses all gaps and concerns from the audit so that PEAL matches the Cursor Orchestrator PRD in letter and spirit. It is organized into four phases: High, Medium, and Low priority (audit gaps), and a fourth phase for error tolerance and robustness so that PRD-to-code runs can tolerate LLM and user unpredictability with minimal intervention. Each task is an actionable subphase.

## Phase 1 — High Priority

## Task 1

**Cursor CLI contract.** Verify prompt passing (`-p` vs positional) and plan mode (`--plan` / `--mode=plan`) against current Cursor CLI docs (e.g. Cursor CLI Overview). Add a one-line comment in `src/phase.rs` citing the doc so future changes do not drift. Resolve omit `--model` vs `--model auto`: either update `src/phase.rs` to only add `--model` when `config.model.is_some()`, or update the PRD to state that Auto is passed as `--model auto`. Document the chosen contract in docs (e.g. `docs/configuration.md` or a short "Cursor CLI contract" section).

## Task 2

**stet_commands.** Either implement: when `stet_commands` is non-empty, run that sequence (or one wrapper command) instead of the built-in `stet start` / `stet run` / `stet finish` path (e.g. via `src/subprocess.rs` `run_command_string` per command in the repo). Define clear semantics (when to run, CWD, timeout) and document. Or remove `stet_commands` from `src/config.rs`, `src/cli.rs`, and user-facing docs, and document it as reserved/unused for v1. Update PRD §9 accordingly.

## Task 3

**Default on_findings_remaining.** Align default with PRD: change `src/config.rs` `DEFAULT_ON_FINDINGS_REMAINING` to `"warn"` and document in `docs/configuration.md` and PRD §5 as "warn and continue". Or keep `"fail"` and update the PRD to state explicitly that the implementation default is "fail" with rationale; document in one place so PRD and code agree on what "default" means.

## Task 4

**Logging and secrets.** In `src/phase.rs` (and any other place that logs argv), redact or truncate the prompt argument when logging (e.g. log `"<prompt len=N>"` instead of the body), or document that debug logs may contain full prompts and must be treated as sensitive. Satisfy PRD §13: "Logs SHALL NOT contain secrets (e.g. full prompt text MAY be truncated or omitted)."

## Task 5

**Rust edition and minimum version.** Confirm `edition = "2024"` in `Cargo.toml` is supported by the minimum supported Rust version (e.g. 1.93+). In README or docs (e.g. contributing), state the minimum Rust version; optionally add `rust-toolchain.toml` or CI to pin it. If the minimum does not support 2024, change `Cargo.toml` to `edition = "2021"` until the project is ready to depend on a newer compiler.

## Phase 2 — Medium Priority

## Task 6

**Resume implicit.** In `docs/cursor-orchestrator-prd.md` §7 Flow 2, clarify that resume is implicit: e.g. "User runs the same command again (same plan path and repo path); the orchestrator resumes automatically when it finds matching state." Optionally add a short note in `src/cli.rs` help for the `run` subcommand that re-running with the same `--plan` and `--repo` resumes from the last completed task.

## Task 7

**State file location.** Document in `docs/configuration.md` or a state section: single state file in `state_dir` (default `.peal`); context matched by `plan_path` + `repo_path` inside the file; mismatch causes state to be discarded so cross-run reuse is prevented. Align wording with PRD §10.

## Task 8

**Document defaults in one place.** Add one table or section (e.g. in `docs/configuration.md`) listing defaults: e.g. `on_findings_remaining`, `max_address_rounds`, `state_dir`, `phase_timeout_sec`, and that "warn and continue" is optional when the default is "fail". Include config precedence: CLI > env > file.

## Task 9

**Platforms NFR.** Add a short "Platforms" or "Distribution" section to README or docs listing supported target triples (e.g. `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`) per PRD §13. Optionally add CI or scripts that build for these targets.

## Task 10

**Config docs and reserved keys.** In user-facing configuration docs, state precedence (CLI > env > file) explicitly. If `stet_commands` remains unused, call it out as reserved or document that it has no effect in v1.

## Task 11

**Windows agent resolution.** Document that on Windows users should pass the full executable name (e.g. `agent.exe`) or path when the agent is not on PATH. Or extend `src/cursor.rs` to resolve using PATHEXT / try `.exe` when the name has no extension.

## Phase 3 — Low Priority

## Task 12

**Normalization beyond v1.** Document in docs or `docs/configuration.md` that plan normalization (`normalize_plan`, `normalize_retry_count`, `normalize_prompt_path`) is an optional extension beyond v1 PRD §4 ("exactly one format"); state is still keyed by `plan_path` + `repo_path` only.

## Task 13

**Optional state fields.** Add a short comment or doc note in `src/state.rs` (or state docs) that `last_plan_by_task` and `last_completed_ref` are reserved for future use (e.g. "re-run phase 2 without phase 1" or ref-based reporting); they are not yet populated by the runner.

## Task 14

**Plan file error message.** Ensure user-facing message for invalid plan file is exactly "Invalid or missing plan file" where required by PRD §4 (or relax the PRD to "clear error"). Check `src/error.rs` and any display paths.

## Task 15

**State format.** Document in `docs/configuration.md` or state section that v1 state format is JSON only (PRD §10 allows TOML or JSON; implementation uses JSON).

## Phase 4 — Error tolerance

## Task 16

**Tolerant vs strict profiles.** Document or implement "tolerant" vs "strict" profiles. For unattended "run and get as much as possible" use: consider defaulting `on_findings_remaining` to `"warn"` and `on_stet_fail` to `"retry_once"` or `"skip"` (or document why current defaults are "strict" and add a "tolerant" profile in `docs/configuration.md`). In `src/config.rs` only change defaults if the decision is to make tolerant the default; otherwise document both profiles in one place.

## Task 17

**Phase 3 retries.** Add retries for the Phase 3 (address findings) Cursor CLI invocation. In `src/config.rs` add a config key (e.g. `phase_3_retry_count` or reuse `phase_retry_count` for P3) with a small cap (e.g. 1–2). In `src/runner.rs` (and any path that calls the address phase), retry the Phase 3 call on timeout or non-zero exit up to that count before failing the task. Ensure state is saved before returning an error.

## Task 18

**Plan-text sanity check after P1.** Optional plan-text validation after Phase 1. In `src/runner.rs` (or a small helper), after capturing P1 stdout, optionally check that plan text length (or structure) is above a configured threshold (e.g. minimum length or presence of expected tokens). If validation fails, retry P1 once (if retries remain) or fail with a clear error (e.g. "Phase 1 returned empty or invalid plan"). Add config (e.g. `validate_plan_text: bool`, `min_plan_text_len: Option<usize>`) if needed; default can be disabled for backward compatibility.

## Task 19

**Run summary and exit semantics.** Record run summary when exit code is 0. Extend state or write a small report (e.g. under `state_dir` or a configurable path) with: tasks completed, tasks failed, tasks with remaining findings. Optionally support a distinct exit code for "completed with issues" (e.g. 0 = all clean, 1 = hard failure, 2 = completed with failures or remaining findings). Implement in `src/main.rs` and `src/runner.rs`; document in `docs/configuration.md`.

## Task 20

**Consecutive-failure cap.** Add config option `max_consecutive_task_failures: Option<u32>`. In `src/runner.rs`, track consecutive task failures (per sequential segment or per task in a block). When the count reaches the cap, stop the run, persist state, and exit with a distinct error (e.g. a new `PealError` variant) so automation can detect "run stopped due to consecutive failures." Add to `src/config.rs`, `src/cli.rs`, and `src/error.rs`; document.

## Task 21

**Document tolerant vs strict profiles.** In `docs/configuration.md` (or equivalent), add a short "Profiles" or "Strict vs tolerant" subsection. Describe **Strict**: current defaults (e.g. `on_findings_remaining = fail`, `on_stet_fail = fail`, no continue-with-remaining). Describe **Tolerant**: for unattended PRD-to-code runs (e.g. `on_findings_remaining = warn`, `on_stet_fail = retry_once` or `skip`, `continue_with_remaining_tasks = true`). List the config keys and example values for each profile so users can copy-paste or reference.

## Task 22

**Log when stet output unparseable.** Where `src/stet.rs` uses `parse_findings_from_run_json` (or equivalent) and receives `None`, add a tracing warning (e.g. `warn!("stet run output was not valid JSON or had no findings array; skipping structured dismiss")`) so operators see that the run is in fallback mode. Do not change behavior; only add the log.
