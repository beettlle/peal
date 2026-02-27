# peal

**Plan–Execute–Address Loop:** an orchestrator that drives the Cursor CLI in three phases per task.

Peal turns a markdown plan into executed work: it invokes the Cursor CLI for **plan** (Phase 1), **execute** (Phase 2), and **address** (Phase 3 — fix review findings). Optional integration with stet runs code review after each task and loops until findings are resolved or dismissed.

---

## Table of Contents

- [Features](#features)
- [Requirements](#requirements)
- [Build and install](#build-and-install)
- [Quick start](#quick-start)
- [Usage](#usage)
- [Plan format](#plan-format)
- [Configuration](#configuration)
- [Exit codes](#exit-codes)
- [Documentation](#documentation)
- [License](#license)

---

## Features

- **Three-phase loop per task** — Plan (Cursor in plan mode) → Execute (Cursor in agent mode) → Address (stet review + agent to fix findings, repeat until clear or max rounds).
- **Cursor CLI integration** — Uses the Cursor CLI (`agent`) for plan creation and execution; no custom agent runtime.
- **Optional stet integration** — When stet is on PATH (or configured), Phase 3 runs `stet start` / `stet run` and addresses findings via the agent; supports dismiss reasons and triage.
- **Resume by state** — State is keyed by plan path and repo path; re-run with the same `--plan` and `--repo` to resume from the last completed task.
- **Plan normalization** — Use `--normalize` to convert PRDs or free-form docs into the canonical plan format via one Cursor CLI call before parsing.
- **Parallel tasks** — Mark tasks with ` (parallel)` in the plan; peal can run consecutive parallel tasks concurrently (configurable concurrency).
- **Configurable behavior** — Config file (TOML), environment variables (`PEAL_*`), and CLI flags; precedence: CLI > env > file > defaults. Strict (fail on findings/stet failure) or tolerant (warn, retry, continue) profiles.

---

## Requirements

- **Rust** 1.93+ (see [Cargo.toml](Cargo.toml) `rust-version`).
- **Cursor CLI** — The `agent` (or configured) binary on PATH; see [Cursor CLI docs](https://docs.cursor.com/context/cli-overview).
- **stet** (optional) — For Phase 3 code review; if not found, Phase 3 is skipped.

---

## Build and install

From the repository root:

```bash
cargo build --release
```

Run directly:

```bash
cargo run --release -- run --plan plans/my-plan.md --repo /path/to/repo
```

Or install the binary (e.g. into `~/.cargo/bin`):

```bash
cargo install --path .
```

See [docs/building.md](docs/building.md) for build requirements, release builds, and supported platforms.

---

## Quick start

1. **Create a plan file** (canonical format: `## Task 1`, `## Task 2`, …; optional ` (parallel)` suffix). Or use `peal prompt --output plan-prompt.txt` to get a template you can send to an LLM.

2. **Run the orchestrator:**

   ```bash
   peal run --plan plans/my-plan.md --repo /path/to/your/repo
   ```

3. **Optional:** Use a config file (e.g. `peal.toml`) or env vars for `plan_path` and `repo_path` so you can run:

   ```bash
   peal run --config peal.toml
   ```

4. **Resume:** Run the same command again; peal resumes from the last completed task (state in `.peal/state.json` by default).

---

## Usage

### Commands

| Command | Description |
|--------|-------------|
| `peal run` | Run the orchestrator: load plan, run phases 1–2–3 per task, optionally stet and address findings. |
| `peal prompt` | Print the plan-format prompt template (for LLMs). Use `--output <path>` to write to a file. |

### Run options (summary)

- **Required:** `--plan <path>`, `--repo <path>` (or set via config / `PEAL_PLAN_PATH`, `PEAL_REPO_PATH`).
- **Config:** `--config <path>` to a TOML file.
- **Agent:** `--agent-cmd <name|path>` (default `agent`), `--model <model>`.
- **State and resume:** `--state-dir <path>` (default `.peal`), `--task <N>` (single task), `--from-task <N>` (from task N to end).
- **Stet:** `--stet-path <path>`, `--stet-start-ref <ref>`, `--on-stet-fail fail|retry_once|skip`, `--max-address-rounds <N>`.
- **Behavior:** `--on-findings-remaining fail|warn`, `--continue-with-remaining-tasks`, `--parallel`, `--max-parallel <N>`.
- **Plan:** `--normalize` to normalize non-canonical input via Cursor CLI before parsing.

Full option list: `peal run --help`. All run options can be set in config or via `PEAL_*` env vars; see [Configuration](#configuration).

---

## Plan format

Plans are markdown with **task headings** `## Task 1`, `## Task 2`, … (optional suffix ` (parallel)` for concurrent execution). Example:

```markdown
## Task 1
Implement the user login flow and add unit tests.

## Task 2 (parallel)
Update the README with setup instructions.

## Task 3 (parallel)
Add a CI job for the new tests.
```

- Preamble before `## Task 1` is allowed and ignored by the parser.
- Use `peal prompt` (or `peal prompt --output ...`) to get a template that describes this format for an LLM.
- If the file is not in this canonical form, run with `--normalize` so peal invokes the Cursor CLI once to convert it before parsing.

---

## Configuration

Configuration precedence: **CLI > environment > config file > built-in defaults.**

- **Config file:** Pass with `--config`; no default path. All keys optional except `plan_path` and `repo_path`, which must be set from at least one source.
- **Environment:** Prefix `PEAL_` and UPPER_SNAKE_CASE (e.g. `PEAL_PLAN_PATH`, `PEAL_REPO_PATH`, `PEAL_ON_FINDINGS_REMAINING`).
- **Strict (default):** `on_findings_remaining = "fail"`, `on_stet_fail = "fail"`, `continue_with_remaining_tasks = false` — good for CI and gates.
- **Tolerant:** `on_findings_remaining = "warn"`, `on_stet_fail = "retry_once"` or `"skip"`, `continue_with_remaining_tasks = true` — for unattended or long runs.

Full reference: [docs/configuration.md](docs/configuration.md).

---

## Exit codes

| Code | Meaning |
|------|--------|
| **0** | All tasks completed; no failures; no remaining findings (Phase 3 resolved or N/A). |
| **1** | Hard failure: config/plan error, phase failure, stet start/run failure, or findings remaining when `on_findings_remaining = "fail"`. |
| **2** | Run finished but with issues: at least one task failed (with `continue_with_remaining_tasks`) or at least one task has remaining findings. Run summary still written. |
| **3** | Run stopped because consecutive task failures reached `max_consecutive_task_failures`; state persisted. |

---

## Platforms / distribution

Supported targets: `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`. Single binary per platform; no Python/Node at runtime. See [docs/configuration.md](docs/configuration.md#supported-platforms-and-targets) for details.

---

## Documentation

- [Building](docs/building.md) — Build requirements, platforms, build-all-targets script.
- [Configuration](docs/configuration.md) — Config file, supported platforms, Cursor CLI contract.

---

## License

MIT
