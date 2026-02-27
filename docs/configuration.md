# Configuration reference

Peal is a plan–execute–address loop orchestrator that drives the Cursor CLI. Configuration is optional: you can pass only `--plan` and `--repo` and rely on defaults. Use a config file, environment variables, or CLI flags to override; all keys are optional except `plan_path` and `repo_path`, which must be set from at least one source. **Precedence is: CLI over environment over config file over built-in defaults.** For each key, the first non-empty value in that order wins.

---

## Precedence

Configuration is built from three layers with **CLI > env > config file > built-in defaults**. For each key, the first non-empty value in that order wins. Implemented in `config.rs` via `merge_layers(file_layer, env_layer, cli_layer)`.

**Example:** If `state_dir` is set to `".peal"` in the config file, `PEAL_STATE_DIR=/var/peal` in the environment, and `--state-dir ./mystate` on the command line, the effective value is `./mystate` (CLI wins).

---

## Defaults at a glance

Configuration precedence: **CLI > env > config file > built-in defaults**. See [Precedence](#precedence) for details.

| Key | Default | Note |
|-----|---------|------|
| `on_findings_remaining` | `"fail"` | Set to `"warn"` for warn-and-continue when findings remain. |
| `on_stet_fail` | `"fail"` | Set to `"retry_once"` or `"skip"` for tolerant behavior on stet failure. |
| `max_address_rounds` | `5` | |
| `state_dir` | `".peal"` | Relative to process cwd unless overridden. |
| `phase_timeout_sec` | `1800` | |

These defaults implement the **strict** profile; see [Tolerant vs strict profiles](#tolerant-vs-strict-profiles) for a **tolerant** profile.

When the default is **fail**, you can optionally set `on_findings_remaining` to **warn** to get warn-and-continue behavior.

For all keys and sources (TOML, env, CLI), see [Configuration keys](#configuration-keys).

---

## Exit codes

The `run` command uses the following exit codes so that scripts and CI can distinguish outcomes:

| Exit code | Meaning |
|-----------|--------|
| **0** | All planned tasks completed; no task failures; no tasks with remaining findings (phase 3 resolved or N/A). |
| **1** | Hard failure: config/plan error, phase failure (or phase 3 findings-remaining when `on_findings_remaining = "fail"`), stet start/run failure, etc. No run summary is written. |
| **2** | Run completed without hard failure but with **issues**: at least one task failed (e.g. with `continue_with_remaining_tasks`) **or** at least one task has remaining findings (phase 3 ran and `findings_resolved == false`). Run summary is written. |
| **3** | Run stopped because the number of consecutive task failures reached `max_consecutive_task_failures`. State was persisted; automation can detect this condition by exit code 3. |

Exit code **2** is useful for CI/scripts to distinguish "all clean" (0) from "done but with failures or remaining findings" (2). The `prompt` command uses only 0 (success) or 1 (failure); no summary and no exit 2.

---

## Tolerant vs strict profiles

Two named profiles help you choose how peal behaves when findings remain, stet fails, or a task fails:

| Profile | `on_findings_remaining` | `on_stet_fail` | `continue_with_remaining_tasks` | Use case |
|---------|-------------------------|----------------|----------------------------------|----------|
| **Strict** (built-in default) | `"fail"` | `"fail"` | `false` (default) | CI, gate checks; fail if anything is left; do not continue after a task failure. |
| **Tolerant** | `"warn"` | `"retry_once"` or `"skip"` | `true` | Unattended PRD-to-code or long runs; get as much done as possible; continue with remaining tasks after a task failure. |

### Strict profile

- **Description:** Built-in defaults: fail when findings remain, fail on stet failure, and do not continue with remaining tasks after a task failure.
- **Config keys and example values** (for reference; these are the defaults when keys are omitted, so no config is required for Strict):
  - `on_findings_remaining = "fail"`
  - `on_stet_fail = "fail"`
  - `continue_with_remaining_tasks = false` (default)
- These are the **built-in defaults** when the keys are omitted. Optional minimal TOML for copy-paste reference:

```toml
on_findings_remaining = "fail"
on_stet_fail = "fail"
continue_with_remaining_tasks = false
```

### Tolerant profile

- **Description:** For unattended PRD-to-code or long runs: warn when findings remain, retry stet once or skip on stet failure, and continue with remaining tasks after a task failure.
- **Config keys and example values** (for copy-paste or reference):
  - `on_findings_remaining = "warn"`
  - `on_stet_fail = "retry_once"` or `"skip"` — use `retry_once` for transient stet failures; use `skip` to always continue without failing on stet.
  - `continue_with_remaining_tasks = true`
- **Copy-pastable snippet** (minimal; you can merge these keys into an existing config):

```toml
plan_path = "plans/my-plan.md"   # or set via --plan / PEAL_PLAN_PATH
repo_path = "/path/to/repo"      # or set via --repo / PEAL_REPO_PATH

on_findings_remaining = "warn"
on_stet_fail = "retry_once"      # or "skip" to always continue on stet failure
continue_with_remaining_tasks = true
```

Built-in defaults correspond to the **strict** profile; use the tolerant snippet above (or equivalent env/CLI) for unattended runs.

---

## Configuration keys

Every key that affects `PealConfig` is listed below. Required keys must be provided via at least one source (CLI, env, or file).

| Key | TOML key | Env var (prefix `PEAL_`) | CLI flag | Type | Default |
|-----|----------|--------------------------|----------|------|---------|
| `agent_cmd` | `agent_cmd` | `AGENT_CMD` | `--agent-cmd` | string | `"agent"` |
| `plan_path` | `plan_path` | `PLAN_PATH` | `--plan` | path | **(required)** |
| `repo_path` | `repo_path` | `REPO_PATH` | `--repo` | path | **(required)** |
| `stet_commands` | `stet_commands` | `STET_COMMANDS` (comma-sep) | — | list of strings | `[]` |
| `sandbox` | `sandbox` | `SANDBOX` | `--sandbox` | string | `"disabled"` |
| `model` | `model` | `MODEL` | `--model` | string | — |
| `max_address_rounds` | `max_address_rounds` | `MAX_ADDRESS_ROUNDS` | `--max-address-rounds` | u32 | `5` |
| `on_findings_remaining` | `on_findings_remaining` | `ON_FINDINGS_REMAINING` | `--on-findings-remaining` | `"fail"` \| `"warn"` | `"fail"` |
| `state_dir` | `state_dir` | `STATE_DIR` | `--state-dir` | path | `".peal"` |
| `phase_timeout_sec` | `phase_timeout_sec` | `PHASE_TIMEOUT_SEC` | `--phase-timeout-sec` | u64 | `1800` |
| `phase_retry_count` | `phase_retry_count` | `PHASE_RETRY_COUNT` | `--phase-retry-count` | u32 | `0` |
| `phase_3_retry_count` | `phase_3_retry_count` | `PHASE_3_RETRY_COUNT` | `--phase-3-retry-count` | u32 | `0` |
| `parallel` | `parallel` | `PARALLEL` (bool) | `--parallel` | bool | `false` |
| `max_parallel` | `max_parallel` | `MAX_PARALLEL` | `--max-parallel` | u32 | `4` |
| `continue_with_remaining_tasks` | `continue_with_remaining_tasks` | `CONTINUE_WITH_REMAINING_TASKS` | `--continue-with-remaining-tasks` | bool | `false` |
| `log_level` | `log_level` | `LOG_LEVEL` | `--log-level` | string | — |
| `log_file` | `log_file` | `LOG_FILE` | `--log-file` | path | — |
| `stet_path` | `stet_path` | `STET_PATH` | `--stet-path` | path | — |
| `stet_start_ref` | `stet_start_ref` | `STET_START_REF` | `--stet-start-ref` | string | — |
| `stet_start_extra_args` | `stet_start_extra_args` | `STET_START_EXTRA_ARGS` | `--stet-start-args` | list | `[]` |
| `stet_run_extra_args` | `stet_run_extra_args` | `STET_RUN_EXTRA_ARGS` | `--stet-run-args` | list | `[]` |
| `stet_disable_llm_triage` | `stet_disable_llm_triage` | `STET_DISABLE_LLM_TRIAGE` | `--stet-disable-llm-triage` | bool | `false` |
| `stet_dismiss_patterns` | `stet_dismiss_patterns` | `STET_DISMISS_PATTERNS` | — | array of `{pattern, reason}` | `[]` |
| `on_stet_fail` | `on_stet_fail` | `ON_STET_FAIL` | `--on-stet-fail` | `"fail"` \| `"retry_once"` \| `"skip"` | `"fail"` |
| `post_run_commands` | `post_run_commands` | `POST_RUN_COMMANDS` (comma-sep) | `--post-run-commands` | list of strings | `[]` |
| `post_run_timeout_sec` | `post_run_timeout_sec` | `POST_RUN_TIMEOUT_SEC` | `--post-run-timeout-sec` | u64 | — |
| `normalize_plan` | `normalize_plan` | `NORMALIZE_PLAN` (bool) | `--normalize` | bool | `false` |
| `normalize_retry_count` | `normalize_retry_count` | `NORMALIZE_RETRY_COUNT` | `--normalize-retries` | u32 | `0` |
| `normalize_prompt_path` | `normalize_prompt_path` | `NORMALIZE_PROMPT_PATH` | — | path | — |
| `validate_plan_text` | `validate_plan_text` | `VALIDATE_PLAN_TEXT` (bool) | `--validate-plan-text` | bool | `false` |
| `min_plan_text_len` | `min_plan_text_len` | `MIN_PLAN_TEXT_LEN` (u64) | `--min-plan-text-len` | u64 | — |
| `run_summary_path` | `run_summary_path` | `RUN_SUMMARY_PATH` | `--run-summary-path` | path | — |
| `max_consecutive_task_failures` | `max_consecutive_task_failures` | `MAX_CONSECUTIVE_TASK_FAILURES` | `--max-consecutive-task-failures` | u32 (optional) | — (not set = no cap) |

**Notes:**

- **Required:** `plan_path` and `repo_path` must be set via any combination of CLI, env, or config file.
- **Env parsing:**  
  - `STET_COMMANDS` and `POST_RUN_COMMANDS`: comma-separated; surrounding whitespace is trimmed.  
  - `STET_DISMISS_PATTERNS`: comma-separated `pattern|reason` pairs (e.g. `generated|out_of_scope, false positive|false_positive`). Invalid or malformed entries are skipped.  
  - Extra-args env vars (`STET_START_EXTRA_ARGS`, `STET_RUN_EXTRA_ARGS`): split on comma and whitespace.
- **`stet_dismiss_patterns`:** Valid `reason` values: `false_positive`, `already_correct`, `wrong_suggestion`, `out_of_scope`. In TOML, use an array of tables with `pattern` and `reason` keys. There is no CLI flag; use TOML or env only.
- **Config file:** Pass the path with `--config`. If `--config` is not set, no file is loaded.
- **`agent_cmd` (Windows):** Resolution looks for the exact name in PATH. On Windows, bare names without an extension are resolved using `.exe` (e.g. `agent` → `agent.exe`). If the command is not found, use the full executable name (e.g. `agent.exe`) or an absolute path to the executable.

---

## Plan normalization

Plan normalization is an optional extension beyond v1 PRD §4 ("exactly one format"): the PRD requires the orchestrator to support exactly one plan format for unambiguous parsing; when normalization is enabled, non-canonical input (e.g. PRDs, notes) can be converted to that format via one Cursor CLI invocation. State and resume are unchanged: state is keyed only by `plan_path` and `repo_path` (see [State and resume](#state-and-resume) below).

When the plan file does **not** match the canonical format (e.g. no `## Task N` headings), and `normalize_plan` or `--normalize` is true, peal invokes the Cursor CLI **once** with the file content and instructions to convert it to canonical format; the agent's stdout is then parsed as the plan.

- **When to use `--normalize` / `normalize_plan = true`:**
  - **Arbitrary input:** PRDs, implementation plans, notes, or other free-form docs you want turned into a PEAL plan in one shot.
- **When not to use:**
  - **Already canonical:** If the file already has `## Task 1`, `## Task 2`, … headings, peal detects that and parses directly; no agent call. Adding `--normalize` does nothing in that case (no extra invocation).
- **Precedence:** CLI `--normalize` overrides env and file; same as other options. `normalize_retry_count` (or `--normalize-retries`) sets how many extra attempts to run normalize+parse on parse failure (default 0).

**Custom normalization prompt:** If `normalize_prompt_path` is set (TOML or `PEAL_NORMALIZE_PROMPT_PATH`), peal reads that file and uses its content as the full normalization prompt. A single placeholder `{{DOC}}` in the file is replaced by the plan document content. If unset, the built-in normalization prompt is used. The path may be absolute or relative to the process current working directory. If the file is missing or unreadable, normalization fails with a clear error.

---

## Plan-text validation

When `validate_plan_text` is **true** (default: **false**), peal validates Phase 1 stdout (plan text) after each successful P1 run: the plan text must be non-empty and, if `min_plan_text_len` is set, at least that many characters. This is off by default for backward compatibility. On the first validation failure, peal retries Phase 1 **once** and re-validates; if it still fails, the run fails with `Phase1PlanTextInvalid` (task index and detail in the error message). This single validation retry is independent of `phase_retry_count`, which applies only to process failure (timeout or non-zero exit) inside the phase layer.

---

## State and resume

Resume uses the **plan actually run**: that is, the parsed plan used for that run — either the file content (when canonical) or the **normalized output** from the single normalization invocation. State is keyed only by `plan_path` and `repo_path`; the content (file vs normalized) is not stored in state. State keying remains `plan_path` + `repo_path` only, per PRD §10. State file location and context matching (single file, mismatch → discard) are described in **Default state path** below.

**Re-normalizing:** If you run again with the same `--plan` and `--repo` but with normalization enabled (or with a modified source file), the LLM may produce different normalized output. Task identity (Task 1, Task 2, …) and count can change. Resuming will still match on `plan_path` and `repo_path` and skip by **task index**; those indices may no longer correspond to the same logical tasks. So if you re-normalize, treat it as a new run: consider clearing state (e.g. remove `.peal/state.json`) or using a different `state_dir` if you need a clean resume.

---

## Edge cases and phase behavior

- **Phase timeout:** When a phase (1 or 2) exceeds `phase_timeout_sec`, the task is failed, state is persisted, and the process exits non-zero. Retries are controlled by `phase_retry_count` (default 0).
- **Phase retry:** `phase_retry_count` (default 0) sets how many extra attempts each of phase 1 and phase 2 gets on timeout or non-zero exit before the task fails. For example, `phase_retry_count = 1` allows one retry per phase.
- **Phase 3 retry:** `phase_3_retry_count` (default 0) sets how many extra attempts Phase 3 (address findings) and the triage step get on timeout or non-zero exit; effective retries are capped at 2 (so at most 3 total attempts). Values &gt; 2 in config/env/CLI are accepted but capped when used.
- **Findings remaining:** Default for `on_findings_remaining` is **fail** (strict-by-default); set to `warn` for warn-and-continue. See [Defaults at a glance](#defaults-at-a-glance) and [Tolerant vs strict profiles](#tolerant-vs-strict-profiles) for a tolerant profile (e.g. unattended runs). PRD §5 describes behavior; the Configuration keys table is the source of truth for the default value.
- **Stet failure:** When stet is used and `stet start` or `stet run` fails, `on_stet_fail` controls behavior: `"fail"` (default) fails the run or task; `"retry_once"` retries once then fails; `"skip"` logs a warning and continues without stet (for start) or marks that task's phase 3 as skipped (for run). `stet finish` remains best-effort (warn on failure). See [Tolerant vs strict profiles](#tolerant-vs-strict-profiles) for a tolerant profile (e.g. unattended runs).

- **Consecutive task failure cap:** When `max_consecutive_task_failures` is set, the runner maintains a single run-wide counter of consecutive task failures. Any task success resets the counter to zero; any task failure increments it. Skipping an already-completed task does not change the counter. When the count reaches the cap, the run stops, state is saved, and the process exits with exit code **3** so automation can detect "run stopped due to consecutive failures" without parsing stderr. In parallel blocks, outcomes are applied in **segment (task) order** for the purpose of the consecutive counter.

---

## Custom stet command sequence (`stet_commands`)

When `stet_commands` is **non-empty**, it replaces the built-in stet sequence (`stet start` / `stet run` / `stet finish`). Use this for wrapper scripts or custom workflows.

| Aspect | Behavior |
|--------|--------|
| **Session start** | Once at the beginning of the run (before any task). **All** entries in `stet_commands` are run in order. CWD = `repo_path`. Timeout = `phase_timeout_sec` per command. `on_stet_fail` applies: first failure fails the run, retries once, or skips phase 3 for the run (per policy). |
| **Per-task run** | Before the address loop for each task, **only the last** command in `stet_commands` is run. That command is expected to produce findings output (e.g. JSON). Stdout/stderr are captured and parsed with the same findings heuristic as built-in `stet run`. After each address round, only that last command is re-run (no new session start). |
| **Finish** | Peal does **not** call `stet finish` when using `stet_commands`. Add `stet finish` (or your cleanup command) to `post_run_commands` if your workflow needs it. |
| **CWD** | Always `config.repo_path`. |
| **Timeout** | `phase_timeout_sec` per command. |
| **Empty list** | Treated as "use built-in": if `stet_path` is set, the built-in `stet start` / `stet run` / `stet finish` sequence is used. |
| **Single command** | Session start and per-task run both execute that one command (e.g. a script that does start+run). |

Example: `stet_commands = ["stet start HEAD~1", "stet run"]` runs both at session start; per task only `stet run` is run (and re-run after each address round).

---

## Default state path

- **Single state file:** There is exactly **one** state file per `state_dir`: `{state_dir}/state.json`. The PRD §10 (State and Resume) describes this as a single state file per (plan path + repo path) pair; the implementation uses one file and stores **context** inside it (see below).
- **State format (v1):** PRD §10 allows the state file to be TOML or JSON; in v1 the implementation uses **JSON only** (file name `state.json`, read/write as JSON).
- **Default state directory:** `state_dir` defaults to `.peal`, interpreted relative to the process current working directory unless overridden. So the **default state path is `.peal/state.json`** (relative to cwd).
- **Context matching:** The state file holds `plan_path` and `repo_path` (and `completed_task_indices`, etc.). The optional state fields `last_plan_by_task` and `last_completed_ref` are reserved for future use and are not yet written by the runner. On load, the orchestrator compares those values to the current run's `--plan` / `--repo` (or config equivalents). If the current run's `plan_path` or `repo_path` **do not match** the values stored in the file, the loaded state is **discarded** and the run starts from task 1 (no resume). This prevents cross-run reuse across different plans or repos; semantics match PRD §10: missing or corrupted state → no resume, start from task 1 and warn the user — and likewise on context mismatch.
- **Path mechanics:** `state_dir` may be absolute or relative. State is written under that directory; `state.json` is created there (see `state.rs`: `PealState::state_file_path` and `save_state`). The directory is created if it does not exist.

---

## Run summary

When a **run** completes successfully (exit 0 or 2), peal writes a small JSON report so that scripts and tools can see what completed, what failed, and what had remaining findings.

- **When it is written:** Only when the run command returns successfully (exit 0 or 2). Not written on hard failure (exit 1) or for the `prompt` command.
- **Where it is written:** By default `{state_dir}/run_summary.json`. You can override the path with `run_summary_path` (TOML, `PEAL_RUN_SUMMARY_PATH`, or `--run-summary-path`).
- **Contents:** `tasks_completed` (indices that completed with no failure and no remaining findings), `tasks_failed` (indices that failed when `continue_with_remaining_tasks` is true), `tasks_with_remaining_findings` (indices where phase 3 ran but findings were not resolved). Optional fields: `exit_code`, `plan_path`, `repo_path`, `completed_at` (ISO8601).

If writing the summary file fails, peal logs a warning and still exits 0 or 2 as determined by the run outcome.

In [Tolerant vs strict profiles](#tolerant-vs-strict-profiles), exit 2 can occur in tolerant runs when findings remain or when tasks are skipped after failure with `continue_with_remaining_tasks`.

---

## Logging and security

Logs do not contain full prompt text (PRD §13). When debug logging is enabled, the prompt argument in phase argv is emitted only as `<prompt len=N>` so that command shape and argument count remain visible without leaking prompt content.

---

## Full TOML example

All keys in the config file are optional. Unknown keys are rejected (`deny_unknown_fields`). Below is a copy-pastable reference example with required keys and a representative set of optional keys.

```toml
# Required (or provide via --plan / --repo or PEAL_PLAN_PATH / PEAL_REPO_PATH)
plan_path = "plans/my-plan.md"
repo_path = "/path/to/repo"

# Optional: Cursor CLI and execution
agent_cmd = "agent"
state_dir = ".peal"
phase_timeout_sec = 1800
phase_retry_count = 0
phase_3_retry_count = 0
parallel = true
max_parallel = 4
on_findings_remaining = "fail"

# Optional: stet integration
stet_commands = ["stet start HEAD~1", "stet run"]
stet_start_extra_args = ["--allow-dirty"]
stet_run_extra_args = ["--verify", "--context", "256k"]
stet_disable_llm_triage = false
stet_dismiss_patterns = [
  { pattern = "generated", reason = "out_of_scope" },
  { pattern = "false positive", reason = "false_positive" }
]

# Optional: stet failure behavior
on_stet_fail = "fail"

# Optional: post-run commands (e.g. stet finish)
post_run_commands = ["stet finish", "echo done"]
post_run_timeout_sec = 60
```

---

## Supported platforms and targets

- **OS:** Windows (native), macOS (x86_64 and ARM64 where Cursor supports), Linux (x86_64; WSL where applicable).
- **Rust target triples:**  
  `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`.
- **Distribution:** Single binary per platform; no runtime dependency on Python or Node for the orchestrator.
- Cursor CLI support per OS/arch is determined by Cursor; peal runs wherever the Cursor CLI runs.
- On macOS, note that `/tmp` is often a symlink to `/private/tmp`; paths may resolve accordingly.

---

## Cursor CLI contract

Peal invokes the Cursor CLI (e.g. `agent`) in an exec-style manner (no shell). The following contract is the single place to update when the Cursor CLI changes.

**Source of truth:** [Cursor CLI Overview](https://docs.cursor.com/context/cli-overview) (or equivalent official docs).

| Aspect | Behavior |
|--------|--------|
| **Prompt** | Passed as the **final positional argument** in the argv. Peal does not use `-p` / `--prompt` unless the official CLI docs specify it. |
| **Plan mode** | Phase 1 and the normalization invocation use **`--plan`**. Phases 2 and 3 do not (Agent mode). |
| **Model** | When `model` is **unset** in config, peal **omits** `--model` so the Cursor CLI uses its default (Auto). When `model` is set, peal passes `--model <value>`. |
| **Other flags** | `--print`, `--workspace <repo>`, `--output-format text` (phase 1 and normalization), `--sandbox <value>` (phases 2 and 3) are set as in `src/phase.rs` and `src/plan.rs`. Argv is built without a shell (exec-style). |

**Windows:** Resolution looks for the exact name in PATH; bare names without an extension are tried with `.exe` (e.g. `agent` → `agent.exe`). Use `agent.exe` or a full path if the default fails.
