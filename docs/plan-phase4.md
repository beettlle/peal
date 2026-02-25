# Phase 4 — Stet integration

Source: docs/implementation-plan.md. Goal: After Phase 2, when stet is available, run stet then "address findings" with Cursor CLI; loop until no findings or max rounds.

## Task 1

SP-4.1 — Stet detection. Before phase 3: resolve `stet` on PATH (in repo env or system). If not found, skip phase 3 for all tasks (not an error). Config: optional explicit stet path.

## Task 2

SP-4.2 — Start stet session. In repo: run `stet start [ref]`. Config: `stet_start_ref` (e.g. `HEAD~1`). Capture stdout/stderr. Working directory = repo root.

## Task 3

SP-4.3 — Findings heuristic. Decide "findings present": from stet machine-readable output if available, else documented heuristic (e.g. exit code != 0 or pattern in stdout).

## Task 4

SP-4.4 — Address-findings Cursor call. If findings: invoke `agent --print --workspace <repo> ... "Address the following stet review findings. Apply fixes and run tests.\n\n{stet_output}"`. If `suggestion` field is present in findings (from `stet fix` or `--suggest-fixes`), include it in the prompt. Capture output.

## Task 5

SP-4.5 — Re-run and loop. After address: run `stet run` (incremental check). If findings remain and rounds < `max_address_rounds`: repeat address step; else configurable: fail task or warn and continue. Config: `max_address_rounds`, behavior on remaining findings.

## Task 6

SP-4.6 — Wire Phase 3 into runner. After Phase 2 for a task: if stet found, run SP-4.2–SP-4.5; else skip. Integrate with sequential runner (and later with parallel block phase 3).

## Task 7

SP-4.7 — Cleanup. Run `stet finish` to persist state and remove the worktree. Run at the end of the task (or PEAL run).
