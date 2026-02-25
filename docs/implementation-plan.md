# PEAL Implementation Plan

**Source:** [cursor-orchestrator-prd.md](cursor-orchestrator-prd.md)

**Assumptions:**

- **Config format:** TOML (common in Rust projects).
- **State file:** Default `.peal/state.json` (project-relative).
- **Testing:** Unit tests with mocked subprocess by default; optional integration tests with real `agent` (e.g. behind feature flag or env).
- **Subphase size:** Small — one PR, 1–2 days, testable in isolation.

**Cursor CLI (actual flags from `agent help`):**

- Command: `agent`
- Non-interactive: `--print` (not `-p` for prompt; `-p` is `--print`)
- Plan mode: `--mode=plan` or `--plan`
- Workspace: `--workspace <path>` (defaults to cwd)
- Prompt: positional args — e.g. `agent --print --plan --workspace /repo "Create a plan for..."`
- Output: `--output-format text` (with `--print`)
- Sandbox: `--sandbox disabled` when repo is set

The PRD’s “`-p` for prompt” does not match the current CLI; this plan uses the real flags above.

---

## Phase 0 — Project bootstrap

**Goal:** Rust project, config loading, logging. No plan parsing or Cursor invocation yet.

| Subphase | Description | Can run in parallel with |
| -------- | ----------- | ------------------------ |
| **SP-0.1** | **Rust project and CLI skeleton** — `cargo init`, layout (e.g. `src/main.rs`, `src/lib.rs`, `src/cli.rs`, `src/config.rs`). Add dependencies: `clap` (or `argh`) for CLI, `anyhow`/`thiserror` for errors. Subcommands: `run` (required args: plan path, repo path). Exit codes: 0 success, non-zero failure. No behavior beyond parsing and printing help. | — |
| **SP-0.2** | **Config loading** — Config struct with: `agent_cmd`, `plan_path`, `repo_path`, `stet_commands`, `sandbox`, `model`, `max_address_rounds`, `state_dir`, `phase_timeout_sec`, `parallel`, `max_parallel`. Load from file (TOML), then override with env vars, then CLI args. Precedence: CLI > env > file. Document precedence. No behavior yet beyond loading and exposing config. | — |
| **SP-0.3** | **Structured logging** — Log to stderr (or configurable log file). Fields: phase, task index, command run, exit code, duration. Do not log full prompt text (truncate or omit). Use `tracing` or `log` + env filter. | — |

**Parallelism:** SP-0.2 and SP-0.3 can start after SP-0.1 is done.

```mermaid
flowchart LR
  SP01[SP-0.1]
  SP02[SP-0.2]
  SP03[SP-0.3]
  SP01 --> SP02
  SP01 --> SP03
```

**Acceptance:** `cargo run -- run --plan P --repo R` parses and exits; config file is read when present; logs appear on stderr.

---

## Phase 1 — Plan parsing and Phase 1 (Plan) only

**Goal:** Parse plan file, invoke Cursor CLI in plan mode for each task, capture plan text. No Phase 2, no state, no resume.

| Subphase | Description | Can run in parallel with |
| -------- | ----------- | ------------------------ |
| **SP-1.1** | **Plan file parsing** — Read file as UTF-8; reject invalid UTF-8 with "Invalid or missing plan file". Regex: `^## Task\s+(\d+)\s*(\(parallel\))?\s*$` (CRLF → LF). Produce ordered list of `(task_index: u32, task_content: String, parallel: bool)`. Detect parallel blocks: consecutive tasks with `parallel == true` form one block. Preserve task order (ascending index). Gaps allowed. Unit tests: valid headings, (parallel) suffix, body until next heading or EOF, invalid UTF-8. | — |
| **SP-1.2** | **CLI args for run** — `run` requires `--plan` and `--repo` (or from config). No `--resume`, `--task`, or `--from-task` yet. Validate plan path exists and is file; repo path exists and is directory. | SP-1.1 |
| **SP-1.3** | **Subprocess helper (exec-style)** — One module: build argv (no shell), `std::process::Command::new(agent_cmd).args([...]).current_dir(repo_root)`. Take command, args, cwd, timeout (optional). Return stdout + stderr (e.g. `String` + `String` or struct). Timeout and exit code in result. Bounded reads into `String`/`Vec<u8>`. Unit test with mock: e.g. run `echo` or a small stub binary; verify no shell, cwd, timeout. | — |
| **SP-1.4** | **Cursor CLI resolution** — Resolve `agent_cmd` from config (default `agent`). Check presence on PATH before first use; if not found, exit with clear message and link to Cursor CLI install. | SP-1.3 |
| **SP-1.5** | **Phase 1 invocation** — Build prompt: "Create a plan for implementing this task: {task_content}". Invoke: `agent --print --plan --workspace <repo_path> [--output-format text] [--model <model> if configured] "<prompt>"`. Pass prompt as single positional arg (quoted content). Capture stdout as plan text (stderr for logs). Configurable timeout; on timeout or non-zero exit: fail task, exit non-zero (no state yet). Centralized prompt construction (one place). | SP-1.3, SP-1.4 |
| **SP-1.6** | **Run Phase 1 for all tasks** — After parsing plan, loop tasks in order. For each: run Phase 1, capture plan text, log. No Phase 2, no state persistence. Deliverable: `peal run --plan path --repo path` runs Phase 1 for every task and prints/captures plan text. | SP-1.1, SP-1.5 |

**Parallelism:** SP-1.2 can be done with SP-1.1; SP-1.4 and SP-1.5 after SP-1.3; SP-1.6 after SP-1.1 and SP-1.5.

```mermaid
flowchart LR
  subgraph branchA [Branch A]
    SP11[SP-1.1]
    SP12[SP-1.2]
  end
  subgraph branchB [Branch B]
    SP13[SP-1.3]
    SP14[SP-1.4]
    SP15[SP-1.5]
  end
  SP16[SP-1.6]
  SP11 --> SP12
  SP11 --> SP16
  SP13 --> SP14
  SP14 --> SP15
  SP15 --> SP16
```

**Acceptance:** Plan file with `## Task 1`, `## Task 2 (parallel)` etc. is parsed; `agent --print --plan ...` is invoked once per task with correct prompt and workspace; plan text is captured. Unit tests with mocked subprocess. Optional: integration test with real `agent` (behind feature or env).

**Dogfooding:** Not yet (no Phase 2).

---

## Phase 2 — Execute plan (Phase 2) and full sequential loop

**Goal:** For each task, run Phase 1 then Phase 2 (Cursor Agent with plan text). No state, no resume, no Phase 3.

| Subphase | Description | Can run in parallel with |
| -------- | ----------- | ------------------------ |
| **SP-2.1** | **Phase 2 invocation** — Prompt: "Execute the following plan. Do not re-plan; only implement and test.\n\n{plan_text}". Invoke: `agent --print --workspace <repo_path> [--sandbox disabled] [--model ...] "<prompt>"`. CWD = repo root (via `--workspace`). Capture stdout/stderr for logging. Timeout and non-zero exit: fail task, exit non-zero. Same central prompt module; use delimiters for plan text. | — |
| **SP-2.2** | **Sequential runner** — For each task in order: run Phase 1 → capture plan text → run Phase 2 with that plan text. No state; no resume. On any task failure: exit non-zero. | SP-2.1 |
| **SP-2.3** | **Prompt construction hardening** — Single module for all prompts (phase 1, phase 2, later phase 3). Delimiters for task content, plan text, stet output. Pass content as args or temp file (UTF-8); no shell. Document injection risk and construction point. | SP-2.1 |

**Parallelism:** SP-2.2 and SP-2.3 both depend on SP-2.1; they can run in parallel after SP-2.1.

```mermaid
flowchart LR
  SP21[SP-2.1]
  SP22[SP-2.2]
  SP23[SP-2.3]
  SP21 --> SP22
  SP21 --> SP23
```

**Deliverable:** Full run: parse plan → Phase 1 → Phase 2 for every task, in order.

**Dogfooding:** **PEAL is dogfoodable from here.** Use a markdown plan (e.g. "Phase 3: state and resume") and run `peal run --plan ... --repo .` to execute it. No state/resume/Stet required.

---

## Phase 3 — State and resume

**Goal:** Persist state after each task; support resume and "run only task N" / "run from task N".

| Subphase | Description | Can run in parallel with |
| -------- | ----------- | ------------------------ |
| **SP-3.1** | **State schema** — Struct: `plan_path`, `repo_path`, `completed_task_indices: Vec<u32>`. Optional: `last_plan_by_task`, `last_completed_ref`. Serialize to JSON (default state file: `.peal/state.json`). Config: `state_dir` or path relative to plan/repo; document default `.peal/state.json`. | — |
| **SP-3.2** | **State read/write** — Load state from configurable path; if missing or invalid, treat as no resume (warn to stderr). After each successful task, append task index to `completed_task_indices` and write state. On failure: persist current state, exit non-zero. | SP-3.1 |
| **SP-3.3** | **Resume semantics** — On `run` with existing state (same plan path + repo path): load state, skip tasks in `completed_task_indices`, run from smallest index not in set. If next work is a parallel block: run only tasks in that block not in set (parallel block behavior in Phase 5). | SP-3.2 |
| **SP-3.4** | **Single task and range** — CLI: `--task N` (run only task N), `--from-task N` (run from task N to end). State may be updated so resume is consistent. | SP-3.2 |

**Parallelism:** SP-3.3 and SP-3.4 both depend on SP-3.2; they can run in parallel after SP-3.2.

```mermaid
flowchart LR
  SP31[SP-3.1]
  SP32[SP-3.2]
  SP33[SP-3.3]
  SP34[SP-3.4]
  SP31 --> SP32
  SP32 --> SP33
  SP32 --> SP34
```

**Deliverable:** Resume and `--task` / `--from-task` work; state in `.peal/state.json` (or configured path).

**Dogfooding:** Use peal with a multi-task plan; stop after one task; resume and verify only remaining tasks run.

---

## Phase 4 — Stet integration (Phase 3)

**Goal:** After Phase 2, when stet is available: run stet, then "address findings" with Cursor CLI; loop until no findings or max rounds.

| Subphase | Description | Can run in parallel with |
| -------- | ----------- | ------------------------ |
| **SP-4.1** | **Stet detection** — Before phase 3: resolve `stet` on PATH (in repo env or system). If not found, skip phase 3 for all tasks (not an error). Config: optional explicit stet path. | — |
| **SP-4.2** | **Start stet session** — In repo: run `stet start [ref]`. Config: `stet_start_ref` (e.g. `HEAD~1`). Capture stdout/stderr. Working directory = repo root. | SP-4.1 |
| **SP-4.3** | **Findings heuristic** — Decide "findings present": from stet machine-readable output if available, else documented heuristic (e.g. exit code != 0 or pattern in stdout). | SP-4.2 |
| **SP-4.4** | **Address-findings Cursor call** — If findings: invoke `agent --print --workspace <repo> ... "Address the following stet review findings. Apply fixes and run tests.\n\n{stet_output}"`. If `suggestion` field is present in findings (from `stet fix` or `--suggest-fixes`), include it in the prompt. Capture output. | SP-4.3 |
| **SP-4.5** | **Re-run and loop** — After address: run `stet run` (incremental check). If findings remain and rounds < `max_address_rounds`: repeat address step; else configurable: fail task or warn and continue. Config: `max_address_rounds`, behavior on remaining findings. | SP-4.4 |
| **SP-4.6** | **Wire Phase 3 into runner** — After Phase 2 for a task: if stet found, run SP-4.2–SP-4.5; else skip. Integrate with sequential runner (and later with parallel block phase 3). | SP-4.5 |
| **SP-4.7** | **Cleanup** — Run `stet finish` to persist state and remove the worktree. Run at the end of the task (or PEAL run). | SP-4.6 |

**Parallelism:** Sequential chain SP-4.1 → SP-4.2 → SP-4.3 → SP-4.4 → SP-4.5 → SP-4.6 → SP-4.7; no parallel subphases within Phase 4.

```mermaid
flowchart LR
  SP41[SP-4.1]
  SP42[SP-4.2]
  SP43[SP-4.3]
  SP44[SP-4.4]
  SP45[SP-4.5]
  SP46[SP-4.6]
  SP47[SP-4.7]
  SP41 --> SP42
  SP42 --> SP43
  SP43 --> SP44
  SP44 --> SP45
  SP45 --> SP46
  SP46 --> SP47
```

**Deliverable:** Full three-phase flow when stet is present; graceful skip when stet absent. Real stet binary used for development and optional integration tests.

**Dogfooding:** Full PEAL loop with stet to build e.g. Phase 5 (parallel execution).

---

## Phase 5 — Parallel execution

**Goal:** Honor `(parallel)` in plan; run phases 1 and 2 concurrently inside a block; run phase 3 sequentially per task after all phase 2s in the block.

| Subphase | Description | Can run in parallel with |
| -------- | ----------- | ------------------------ |
| **SP-5.1** | **Execution scheduler** — Use parsed parallel blocks from SP-1.1. Execution order: sequential segments and parallel blocks. For a parallel block: identify set of task indices. | — |
| **SP-5.2** | **Concurrent phases 1 and 2** — For a parallel block: spawn one execution stream per task (e.g. `tokio` tasks or threads). Each stream: Phase 1 → Phase 2 for that task. Wait for all streams to finish before starting phase 3. Use safe concurrency (no shared mutable state). | SP-5.1 |
| **SP-5.3** | **Sequential Phase 3 for block** — After all phase 2s in block complete: run phase 3 (stet + address) sequentially for each task in the block (task order). | SP-5.2 |
| **SP-5.4** | **Config and edge cases** — Config: `parallel` (enable/disable), `max_parallel` (cap concurrent Cursor processes). If `parallel` false or `max_parallel == 1`: run all tasks sequentially. Single-task "block": treat as sequential. One task in block fails: persist only completed indices for that block; exit non-zero unless config "continue with remaining tasks". | SP-5.2 |
| **SP-5.5** | **Resume and parallel blocks** — On resume, if next work is a parallel block: run only tasks in block not in `completed_task_indices`; run those in parallel (phases 1+2) then phase 3 sequentially; skip completed tasks in block. | SP-5.3, SP-3.3 |

**Parallelism:** SP-5.2 enables concurrent task execution at runtime; SP-5.4 can be done in parallel with SP-5.3 after SP-5.2. SP-5.5 depends on SP-5.3 and SP-3.3.

```mermaid
flowchart LR
  SP51[SP-5.1]
  SP52[SP-5.2]
  SP53[SP-5.3]
  SP54[SP-5.4]
  SP55[SP-5.5]
  SP51 --> SP52
  SP52 --> SP53
  SP52 --> SP54
  SP53 --> SP55
  SP54 --> SP55
```

**Deliverable:** Plans with `## Task N (parallel)` run with concurrent phases 1+2 and sequential phase 3 per block; resume and edge cases per PRD §14.

---

## Phase 6 — Final review and polish

**Goal:** Post-run commands, config/docs, edge cases per PRD.

| Subphase | Description | Can run in parallel with |
| -------- | ----------- | ------------------------ |
| **SP-6.1** | **Post-run commands** — Optional: after all tasks, run user-configured "final stet review" and "final brutal audit" commands (e.g. config keys). Working dir = repo; capture stdout/stderr; no Cursor call. | — |
| **SP-6.2** | **Config and docs** — Document all config keys, precedence, default state path (`.peal/state.json`), TOML example. Document supported platforms/targets. | — |
| **SP-6.3** | **Edge cases** — Cursor CLI not found, plan missing/unparseable, repo not a git repo, phase timeout, retry (configurable), stet fails (configurable). Per PRD §14 table. | — |

**Parallelism:** SP-6.1, SP-6.2, and SP-6.3 have no dependencies on each other; they can be implemented in parallel.

```mermaid
flowchart LR
  SP61[SP-6.1]
  SP62[SP-6.2]
  SP63[SP-6.3]
```

---

## Summary

- **Config:** TOML; precedence CLI > env > file.
- **State:** Default `.peal/state.json`.
- **Cursor CLI:** `agent --print --plan` / `--workspace`; prompt as positional arg.
- **Stet:** Real binary; phase 3 implemented and tested against it.
- **Testing:** Unit tests with mocked subprocess; optional integration tests with real `agent`/`stet`.
- **Dogfooding:** From **Phase 2** (plan + P1 + P2, no state); then with **Phase 3** (resume); then full loop with **Phase 4** (stet) and **Phase 5** (parallel).
