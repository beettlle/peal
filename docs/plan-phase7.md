# Phase 7 — Plan normalization and flexible input

Source: docs/implementation-plan.md. Goal: Allow users to pass arbitrary documents (PRDs, implementation plans, notes). Optionally normalize via Cursor CLI to the canonical plan format so the rest of the pipeline (Phases 1–6) is unchanged.

## Task 1

SP-7.1 — Format detection. Before parsing: detect whether the plan file matches the canonical format (e.g. regex for `^## Task\s+\d+` or documented phase-table pattern). Document the detection rule. If detected, parse with existing SP-1.1 logic; no Cursor call.

## Task 2

SP-7.2 — Normalization invocation. When format not detected and normalization is enabled (config or CLI): invoke Cursor CLI once with document content + instructions that specify exact output format (canonical `## Task N` / optional `(parallel)`). Capture stdout as normalized plan. Use same `agent_cmd` and workspace as run. Config: `normalize_plan` (bool); CLI: `--normalize`.

## Task 3

SP-7.3 — Validation and retry. Parse normalized output with existing plan parser (SP-1.1). On parse failure: clear error with snippet; optional configurable retry. Ensure single canonical format for all parsing.

## Task 4

SP-7.4 — Config and docs. Document `normalize_plan`, `--normalize`, when to use (arbitrary input vs. already canonical). Optional: config path for normalization prompt or inline prompt template. State/resume: document that resume uses the plan actually run (file or normalized output); if user re-normalizes, task identity may change.

**Implemented.** See `docs/configuration.md`: configuration keys table (`normalize_plan`, `normalize_retry_count`), **Plan normalization** subsection, and **State and resume** subsection.
