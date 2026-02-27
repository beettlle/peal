# Plan: stet_commands — Implement or Remove

**Status:** Option A was implemented. `stet_commands` is used at runtime (session start runs all commands once; per-task runs only the last command). See **docs/configuration.md** § Custom stet command sequence (`stet_commands`) and the configuration keys table.

**Task:** Either implement `stet_commands` (run custom sequence instead of built-in stet start/run/finish) with clear semantics and docs, or remove it from config/CLI/docs and document as reserved for v1. Update PRD §9 accordingly.

**Current state:**
- `stet_commands` is loaded in `src/config.rs` (file + env; no CLI flag in `src/cli.rs`) and stored in `PealConfig`. It **is used at runtime** when non-empty: session start runs the full list once; per-task phase 3 runs only the last command. See `src/main.rs` and runner/stet wiring.
- Built-in stet flow: when `stet_commands` is empty, `main.rs` calls `stet::start_session` once, then `runner::run_scheduled(..., run_stet_path)`. Runner uses `stet::run_review` and `stet::address_loop` per task. After tasks, `main.rs` calls `stet::finish_session`. So: **start (once) → per-task run + address loop → finish (once)**.
- `subprocess::run_command_string(cmd, cwd, timeout)` exists and is used for `post_run_commands` and for custom `stet_commands` (exec-style, CWD = repo, timeout configurable).

---

## Option A — Implement stet_commands

### Goal

When `stet_commands` is non-empty, run that sequence (or one wrapper) **instead of** the built-in `stet start` / `stet run` / `stet finish` path. Use `run_command_string` per command in the repo; define and document when each part runs, CWD, and timeout.

### Semantics to define and document

| Aspect | Decision |
|--------|----------|
| **When session “start” runs** | Once at the beginning of the run (before any task). Run **all** entries in `stet_commands` in order, with CWD = `repo_path`, timeout = `phase_timeout_sec` (or a dedicated `stet_commands_timeout_sec` if added). |
| **When “run” (get findings) runs** | Per task, before the address loop: run **only the last** command in `stet_commands`. That command is assumed to be the “stet run” equivalent (produces findings output). Capture stdout/stderr and reuse existing findings heuristic (JSON parsing). |
| **When “finish” runs** | Do **not** call built-in `stet::finish_session` when using `stet_commands`. User adds `stet finish` (or equivalent) to `post_run_commands` if they want cleanup. Document this. |
| **CWD** | Always `config.repo_path`. |
| **Timeout** | Use `phase_timeout_sec` per command, unless we add `stet_commands_timeout_sec` (optional). |
| **Failure** | Same policy as built-in: `on_stet_fail` applies to (1) session start (running the full list) and (2) per-task run (running the last command). |
| **Mutual exclusivity** | When `stet_commands` is non-empty, do not call `stet::start_session`, `stet::run_review`, or `stet::finish_session`. Use custom path only. |

### Implementation steps

1. **Config**
   - Keep `stet_commands` as-is (already in config).
   - Optionally add `stet_commands_timeout_sec` (default: use `phase_timeout_sec`). If not added, document that `phase_timeout_sec` applies.

2. **Main**
   - After resolving `stet_path` (for logging/optional finish), branch:
     - If `!config.stet_commands.is_empty()`: run each of `config.stet_commands` in order via `run_command_string(cmd, &config.repo_path, Some(timeout))`. Apply `on_stet_fail` to the overall “session start” (e.g. first failure fails or skips per policy). Do **not** set `run_stet_path` for the runner (or pass a marker that phase 3 uses custom commands).
     - Else: current behavior (call `stet::start_session`, set `run_stet_path`).

3. **Runner / phase 3**
   - When phase 3 runs and we are in “custom commands” mode (e.g. `stet_commands` non-empty and no `stet_path` used for built-in):
     - Run **only the last** command in `stet_commands` via `run_command_string`, CWD = `repo_path`, timeout = `phase_timeout_sec`.
     - Capture stdout/stderr into a structure compatible with `stet::StetRunResult` (or add a constructor that takes raw stdout/stderr and runs the existing findings heuristic).
     - Feed that into the existing address loop (same Cursor CLI “address findings” flow). Re-run only the last command after each address round (no new “start”).
   - When phase 3 runs with built-in path: unchanged (current `stet::run_review` etc.).

4. **Finish**
   - When `stet_commands` is non-empty: do **not** call `stet::finish_session` in `main.rs`. Document that user should add `stet finish` to `post_run_commands` if their workflow needs it.

5. **Types / wiring**
   - Introduce a clear phase-3 mode: e.g. `enum StetPhase3Mode { BuiltIn(PathBuf), CustomCommands(Vec<String>) }`. `main` builds this from config and passes to runner. Runner branches on the enum: BuiltIn → current stet path + `stet::run_review`; CustomCommands → run last command string and parse output.

6. **Docs**
   - **configuration.md:** Describe `stet_commands`: when each part runs (start = full list once; per-task = last command only; finish = user’s responsibility via `post_run_commands`). CWD, timeout, and `on_stet_fail` behavior.
   - **PRD §9:** Update “Exact command sequence” to state that when `stet_commands` is set, that sequence replaces the built-in sequence; document “session start = all commands,” “per-task run = last command,” “cleanup = post_run_commands.”

### Edge cases

- **Empty list:** Treated as “use built-in” (current behavior).
- **Single command:** Session start and per-task run both run that one command (acceptable; user can use a wrapper script).
- **stet_path + stet_commands:** Prefer `stet_commands` when non-empty (custom path); ignore `stet_path` for start/run; optionally still use `stet_path` for finish only if we want to support that (or document that with custom commands we don’t call finish).

---

## Option B — Remove stet_commands (reserve for v1)

### Goal

Remove `stet_commands` from runtime config and user-facing docs; document it as reserved/unused for v1 so we don’t promise behavior we don’t implement.

### Implementation steps

1. **src/config.rs**
   - Remove `stet_commands` from `PealConfig`, `FileConfig`, and `ConfigLayer`.
   - Remove from `merge_layers`, `load_file_layer`, `load_env_layer` (and `cli_layer_from` if it were there; currently it’s `None` for stet_commands).
   - Remove all tests that set or assert `stet_commands` (e.g. `documented_example_toml_parses_successfully`, `loads_from_toml_file`, `env_stet_commands_parsed_from_comma_separated`). Adjust TOML examples in tests to drop `stet_commands` line.

2. **src/cli.rs**
   - No change required (there is no `--stet-commands` flag). If any code or comment referred to stet_commands, remove it.

3. **User-facing docs**
   - **docs/configuration.md:** Remove `stet_commands` from the configuration keys table and from the full TOML example. Add a short “Reserved / unused for v1” subsection (or note under stet integration): “The key `stet_commands` is reserved for a future release; in v1 it has no effect and must not be set (or if present it is ignored).” Precedence (CLI > env > file) remains documented elsewhere.

4. **Other code**
   - **src/plan.rs, src/phase.rs, src/runner.rs, src/stet.rs:** Remove `stet_commands: vec![]` (or equivalent) from every test/helper that builds `PealConfig` or a config-like struct. Grep for `stet_commands` and remove the field from struct literals.

5. **PRD §9**
   - In “Exact command sequence,” state that v1 uses only the built-in sequence: `stet start [ref]`, `stet run`, and `stet finish`. Add a sentence: “A config key such as `stet_commands` (or equivalent) is reserved for a future release to allow a custom or wrapper command sequence; in v1 it is unused and has no effect.”

6. **Optional: implementation-plan.md / improvement-plan.md**
   - Update or remove the line that mentions `stet_commands` so the plan reflects the chosen option (implemented vs reserved).

---

## PRD §9 update (both options)

- **If Option A:** In §9 “Exact command sequence,” add: when `stet_commands` is non-empty, that list replaces the built-in sequence; session start = run all commands in order; per-task “run” = run only the last command (expected to produce findings output); cleanup = user’s responsibility via `post_run_commands`. CWD = repo root; timeout = configurable (e.g. `phase_timeout_sec`).
- **If Option B:** In §9, add one sentence that a key like `stet_commands` is reserved for a future release and has no effect in v1.

---

## Recommendation

- **Option A** if the goal is to support custom/wrapper stet workflows soon (e.g. one script that does `stet start && stet run`) and you are willing to maintain the “last command = run” convention and the extra branch in main/runner.
- **Option B** if v1 should stay simple and the built-in sequence is sufficient; reserve `stet_commands` for a later release with a single, clear semantics document.

---

## Verification

- **Option A:** With `stet_commands = ["stet start HEAD~1", "stet run"]`, run peal; confirm session start runs both commands once; per task only the last command runs; no built-in `stet finish`; address loop and findings parsing still work. With `stet_commands = []`, behavior unchanged (built-in path). Unit/integration tests for custom-commands path.
- **Option B:** Build and tests pass; config file with `stet_commands` still parses if we keep the key but ignore it (optional), or we remove the key and document “reserved.” No references to `stet_commands` in config struct or user docs except “reserved for v1.”
