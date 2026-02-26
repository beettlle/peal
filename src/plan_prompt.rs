//! Prompt template for LLMs to produce PEAL-compatible plan markdown.
//!
//! This module holds the static instructions that `peal prompt` prints so
//! users can paste them into an LLM (e.g. Cursor Ask), add their own context,
//! and get back a plan in the canonical format that [`crate::plan`] parses.

/// Return the full prompt template for an LLM to produce a PEAL-compatible plan.
///
/// The output is self-contained: role, format rules, minimal example, and
/// guidance. The user can append their PRD, implementation plan, or notes
/// and ask the LLM to output a single markdown file in the required format.
pub fn plan_instructions_prompt() -> &'static str {
    r#"You are helping produce a feature plan (or implementation plan) that will be executed by PEAL (Plan-Execute-Address Loop), an orchestrator that runs an LLM in plan mode per task, then execute mode, then optional review. The plan must be a single markdown file in the following format so the orchestrator can parse it.

## Required format

- **Task headings:** Use exactly `## Task 1`, `## Task 2`, `## Task 3`, and so on (digit sequence). No other heading style for tasks.
- **Optional parallel marker:** A task heading may include the suffix ` (parallel)`, e.g. `## Task 2 (parallel)`. Consecutive tasks marked `(parallel)` may be run in parallel by the orchestrator; other tasks run in order.
- **Task body:** Everything from the line after a task heading until the next line that matches `## Task N` (or end of file) is that task's content. Use UTF-8.
- **Preamble:** You may include a title, goal, or instructions before `## Task 1`; the parser ignores it. Keep task bodies self-contained and testable.

## Example

```markdown
# Phase X — Short title

Source: docs/spec.md. Goal: One sentence.

## Task 1

First task summary. Body can reference external specs: Full spec: docs/spec.md Section 1.

## Task 2 (parallel)

SP-2.1 — Subphase description. Details here.

## Task 3 (parallel)

SP-2.2 — Another subphase. Consecutive (parallel) tasks can run concurrently.
```

## Guidance

- Keep each task small and testable (e.g. one subphase or one PR-sized unit).
- Task bodies may reference external docs (e.g. "Full spec: docs/implementation-plan.md Phase 3 SP-3.1").
- Use ascending task indices (1, 2, 3, …); gaps are allowed. Only add `(parallel)` when tasks are independent and safe to run concurrently.

Output only the plan markdown. The user will save it to a file and run: `peal run --plan <path> --repo <path>`."#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_task_heading_example() {
        let p = plan_instructions_prompt();
        assert!(
            p.contains("## Task"),
            "template must describe task heading format"
        );
    }

    #[test]
    fn prompt_contains_parallel_marker() {
        let p = plan_instructions_prompt();
        assert!(
            p.contains("(parallel)"),
            "template must describe parallel marker"
        );
    }

    #[test]
    fn prompt_contains_peal_reference() {
        let p = plan_instructions_prompt();
        assert!(
            p.contains("PEAL") || p.contains("peal"),
            "template must mention PEAL"
        );
    }
}
