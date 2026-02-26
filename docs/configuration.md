# Configuration reference

Peal is a plan–execute–address loop orchestrator that drives the Cursor CLI. Configuration is optional: you can pass only `--plan` and `--repo` and rely on defaults. Use a config file, environment variables, or CLI flags to override; all keys are optional except `plan_path` and `repo_path`, which must be set from at least one source.

---

## Precedence

Configuration is built from three layers with **CLI > environment > config file > built-in defaults**. For each key, the first non-empty value in that order wins. Implemented in `config.rs` via `merge_layers(file_layer, env_layer, cli_layer)`.

**Example:** If `state_dir` is set to `".peal"` in the config file, `PEAL_STATE_DIR=/var/peal` in the environment, and `--state-dir ./mystate` on the command line, the effective value is `./mystate` (CLI wins).

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

**Notes:**

- **Required:** `plan_path` and `repo_path` must be set via any combination of CLI, env, or config file.
- **Env parsing:**  
  - `STET_COMMANDS` and `POST_RUN_COMMANDS`: comma-separated; surrounding whitespace is trimmed.  
  - `STET_DISMISS_PATTERNS`: comma-separated `pattern|reason` pairs (e.g. `generated|out_of_scope, false positive|false_positive`). Invalid or malformed entries are skipped.  
  - Extra-args env vars (`STET_START_EXTRA_ARGS`, `STET_RUN_EXTRA_ARGS`): split on comma and whitespace.
- **`stet_dismiss_patterns`:** Valid `reason` values: `false_positive`, `already_correct`, `wrong_suggestion`, `out_of_scope`. In TOML, use an array of tables with `pattern` and `reason` keys. There is no CLI flag; use TOML or env only.
- **Config file:** Pass the path with `--config`. If `--config` is not set, no file is loaded.

---

## Edge cases and phase behavior

- **Phase timeout:** When a phase (1 or 2) exceeds `phase_timeout_sec`, the task is failed, state is persisted, and the process exits non-zero. Retries are controlled by `phase_retry_count` (default 0).
- **Phase retry:** `phase_retry_count` (default 0) sets how many extra attempts each of phase 1 and phase 2 gets on timeout or non-zero exit before the task fails. For example, `phase_retry_count = 1` allows one retry per phase.
- **Stet failure:** When stet is used and `stet start` or `stet run` fails, `on_stet_fail` controls behavior: `"fail"` (default) fails the run or task; `"retry_once"` retries once then fails; `"skip"` logs a warning and continues without stet (for start) or marks that task’s phase 3 as skipped (for run). `stet finish` remains best-effort (warn on failure).

---

## Default state path

- **Default state directory:** `state_dir` defaults to `.peal`, interpreted relative to the process current working directory unless overridden.
- **State file:** The state file is always `{state_dir}/state.json`. So the **default state path is `.peal/state.json`** (relative to cwd).
- `state_dir` may be absolute or relative. State is written under that directory; `state.json` is created there (see `state.rs`: `PealState::state_file_path` and `save_state`). The directory is created if it does not exist.

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
