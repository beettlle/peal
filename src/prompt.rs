//! Centralized prompt construction for all PEAL phases.
//!
//! Every prompt sent to the Cursor CLI is built here so there is exactly
//! one place to review, audit, and test the strings that reach the agent.
//!
//! # Injection risk
//!
//! Task content, plan text, and stet output originate from user-authored
//! files or tool stdout.  An attacker who controls those inputs could craft
//! text that mimics the delimiter or the instruction envelope, causing the
//! agent to interpret data as instructions.
//!
//! Mitigations applied here:
//!
//! 1. **Fenced delimiters** — every dynamic block is wrapped in a
//!    unique `---TAG---` pair so the agent can distinguish instructions
//!    from payload.
//! 2. **Single construction point** — all prompts are built in this
//!    module, making it the only place to audit for prompt structure.
//! 3. **No shell** — prompts are passed as a single positional arg to
//!    `std::process::Command` (via `subprocess::run_command`), never
//!    through a shell, so shell meta-characters have no effect.
//!
//! Residual risk: the delimiters are static strings.  If the payload
//! contains the exact delimiter line the agent *may* be confused.  A
//! future hardening step could use randomised or HMAC-tagged delimiters.

/// Delimiter used to fence task content inside the Phase 1 prompt.
const TASK_DELIMITER: &str = "---TASK---";

/// Delimiter used to fence plan text inside the Phase 2 prompt.
const PLAN_DELIMITER: &str = "---PLAN---";

/// Delimiter used to fence stet review output inside the Phase 3 prompt.
const STET_DELIMITER: &str = "---STET---";

/// Delimiter used to fence extracted suggestions inside the Phase 3 prompt.
const SUGGESTIONS_DELIMITER: &str = "---SUGGESTIONS---";

/// Delimiter used to fence the user document in the normalization prompt (SP-7.2).
const DOC_DELIMITER: &str = "---DOC---";

/// Placeholder in custom normalization prompt files; replaced by the plan document content.
pub const NORMALIZE_PROMPT_PLACEHOLDER: &str = "{{DOC}}";

/// Build the normalization prompt from a custom template string (e.g. from a file).
/// Replaces the single placeholder `NORMALIZE_PROMPT_PLACEHOLDER` with `document_content`.
pub fn build_normalize_prompt_from_template(template: &str, document_content: &str) -> String {
    template.replace(NORMALIZE_PROMPT_PLACEHOLDER, document_content)
}

/// Build the plan normalization prompt (SP-7.2).
///
/// Asks the agent to convert arbitrary document content into the canonical plan format.
/// The user document is fenced with `---DOC---` so it is treated as input, not instructions.
///
/// Output format: headings exactly `## Task 1`, `## Task 2`, …; optional suffix ` (parallel)` per task;
/// task body from the line after the heading until the next `## Task N` or EOF.
pub fn normalize_plan_prompt(document_content: &str) -> String {
    const INSTRUCTIONS: &str = "Convert the following document into a PEAL plan in canonical format.\n\n\
Output rules:\n\
- Use exactly these headings: ## Task 1, ## Task 2, ## Task 3, … (one per task).\n\
- Optionally add \" (parallel)\" after the task number for tasks that can run in parallel (e.g. ## Task 2 (parallel)).\n\
- The task body is everything from the line after the heading until the next ## Task N or end of file.\n\
- Output only the plan markdown (no preamble or explanation).\n\n";
    format!(
        "{INSTRUCTIONS}\
         {DOC_DELIMITER}\n\
         {document_content}\n\
         {DOC_DELIMITER}"
    )
}

/// Build the triage prompt: stet output plus the single question "Anything to address from this review?"
/// Used by Phase 3 auto-dismiss to let the LLM decide which findings to dismiss.
pub fn triage_prompt(stet_output: &str) -> String {
    format!(
        "{STET_DELIMITER}\n\
         {stet_output}\n\
         {STET_DELIMITER}\n\n\
         Anything to address from this review?"
    )
}

/// Build the Phase 1 (plan) prompt for a given task.
///
/// The task content is wrapped in `---TASK---` delimiters so the agent
/// treats it as data, not as additional top-level instructions.
pub fn phase1(task_content: &str) -> String {
    format!(
        "Create a plan for implementing this task:\n\n\
         {TASK_DELIMITER}\n\
         {task_content}\n\
         {TASK_DELIMITER}"
    )
}

/// Build the Phase 2 (execute) prompt for a given plan text.
///
/// The plan text is wrapped in `---PLAN---` delimiters so the agent can
/// distinguish the plan content from the instruction envelope.
///
/// Note: We use `PLAN_DELIMITER` here specifically to distinguish from
/// `TASK_DELIMITER` (Phase 1) and `STET_DELIMITER` (Phase 3). This helps
/// the agent understand the context of the input data.
pub fn phase2(plan_text: &str) -> String {
    format!(
        "Execute the following plan. Do not re-plan; only implement and test.\n\n\
         {PLAN_DELIMITER}\n\
         {plan_text}\n\
         {PLAN_DELIMITER}"
    )
}

/// Build the Phase 3 (address stet findings) prompt.
///
/// Delegates to [`phase3_with_suggestions`] with no suggestions block.
pub fn phase3(stet_output: &str) -> String {
    phase3_with_suggestions(stet_output, None)
}

/// Build the Phase 3 prompt, optionally including an extracted suggestions block.
///
/// When `suggestions` is `Some`, a `---SUGGESTIONS---` fenced block is
/// appended after the stet output so the agent can see machine-extracted
/// fix proposals alongside the raw findings.
pub fn phase3_with_suggestions(stet_output: &str, suggestions: Option<&str>) -> String {
    let mut prompt = format!(
        "Address the following stet review findings. Apply fixes and run tests.\n\n\
         {STET_DELIMITER}\n\
         {stet_output}\n\
         {STET_DELIMITER}"
    );

    if let Some(sugg) = suggestions {
        prompt.push_str(&format!(
            "\n\nSuggested fixes from stet:\n\n\
             {SUGGESTIONS_DELIMITER}\n\
             {sugg}\n\
             {SUGGESTIONS_DELIMITER}"
        ));
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Phase 1 tests --

    #[test]
    fn phase1_wraps_task_in_delimiters() {
        let prompt = phase1("Add a login form with OAuth support.");
        assert!(
            prompt.starts_with("Create a plan for implementing this task:\n\n---TASK---\n"),
            "should start with instruction + delimiter, got: {prompt}"
        );
        assert!(
            prompt.ends_with("\n---TASK---"),
            "should end with closing delimiter, got: {prompt}"
        );
        assert!(
            prompt.contains("Add a login form with OAuth support."),
            "should contain task content verbatim"
        );
    }

    #[test]
    fn phase1_has_exactly_two_task_delimiters() {
        let prompt = phase1("anything");
        let count = prompt.matches("---TASK---").count();
        assert_eq!(count, 2, "exactly two ---TASK--- delimiters expected");
    }

    #[test]
    fn phase1_handles_multiline_content() {
        let content = "Step 1: do X.\nStep 2: do Y.\nStep 3: verify.";
        let prompt = phase1(content);
        assert!(
            prompt.contains(content),
            "multiline task content should appear verbatim"
        );
    }

    #[test]
    fn phase1_handles_empty_content() {
        let prompt = phase1("");
        assert!(prompt.contains("---TASK---\n\n---TASK---"));
    }

    // -- Phase 2 tests --

    #[test]
    fn phase2_wraps_plan_in_delimiters() {
        let prompt = phase2("1. Create file\n2. Write tests");
        assert!(
            prompt.starts_with("Execute the following plan. Do not re-plan; only implement and test.\n\n---PLAN---\n"),
            "should start with instruction + delimiter, got: {prompt}"
        );
        assert!(
            prompt.ends_with("\n---PLAN---"),
            "should end with closing delimiter, got: {prompt}"
        );
        assert!(
            prompt.contains("1. Create file\n2. Write tests"),
            "should contain plan text verbatim"
        );
    }

    #[test]
    fn phase2_handles_empty_plan() {
        let prompt = phase2("");
        assert!(prompt.contains("---PLAN---\n\n---PLAN---"));
    }

    #[test]
    fn phase2_handles_multiline_plan() {
        let plan = "Step 1: scaffold module.\nStep 2: add tests.\nStep 3: integrate.";
        let prompt = phase2(plan);
        assert!(prompt.contains(plan));
        let delimiter_count = prompt.matches("---PLAN---").count();
        assert_eq!(delimiter_count, 2, "exactly two delimiters expected");
    }

    // -- Phase 3 tests --

    #[test]
    fn phase3_wraps_stet_in_delimiters() {
        let prompt = phase3("warning: unused variable `x`\n  --> src/lib.rs:10:9");
        assert!(
            prompt.starts_with(
                "Address the following stet review findings. Apply fixes and run tests.\n\n---STET---\n"
            ),
            "should start with instruction + delimiter, got: {prompt}"
        );
        assert!(
            prompt.ends_with("\n---STET---"),
            "should end with closing delimiter, got: {prompt}"
        );
        assert!(
            prompt.contains("unused variable `x`"),
            "should contain stet output verbatim"
        );
    }

    #[test]
    fn phase3_has_exactly_two_stet_delimiters() {
        let prompt = phase3("finding");
        let count = prompt.matches("---STET---").count();
        assert_eq!(count, 2, "exactly two ---STET--- delimiters expected");
    }

    #[test]
    fn phase3_handles_empty_output() {
        let prompt = phase3("");
        assert!(prompt.contains("---STET---\n\n---STET---"));
    }

    #[test]
    fn phase3_handles_multiline_output() {
        let output = "Finding 1: bad naming\nFinding 2: missing test\nFinding 3: dead code";
        let prompt = phase3(output);
        assert!(prompt.contains(output));
    }

    // -- phase3_with_suggestions tests --

    #[test]
    fn phase3_with_suggestions_includes_block() {
        let prompt = phase3_with_suggestions("finding text", Some("Use X instead"));
        assert!(
            prompt.contains("---SUGGESTIONS---"),
            "should contain SUGGESTIONS delimiter: {prompt}"
        );
        assert!(
            prompt.contains("Use X instead"),
            "should contain suggestion text: {prompt}"
        );
        let sugg_count = prompt.matches("---SUGGESTIONS---").count();
        assert_eq!(sugg_count, 2, "exactly two SUGGESTIONS delimiters expected");
    }

    #[test]
    fn phase3_with_suggestions_none_matches_phase3() {
        let a = phase3("some stet output");
        let b = phase3_with_suggestions("some stet output", None);
        assert_eq!(a, b, "phase3(x) and phase3_with_suggestions(x, None) must be identical");
    }

    #[test]
    fn phase3_with_suggestions_delimiters_distinct() {
        let prompt = phase3_with_suggestions("stet stuff", Some("suggestion"));
        assert!(
            !prompt.contains("---TASK---"),
            "must not contain TASK delimiter"
        );
        assert!(
            !prompt.contains("---PLAN---"),
            "must not contain PLAN delimiter"
        );
        assert!(prompt.contains("---STET---"));
        assert!(prompt.contains("---SUGGESTIONS---"));
    }

    #[test]
    fn phase3_with_suggestions_preserves_stet_block() {
        let stet = "warning: unused `x`\n  --> src/lib.rs:10";
        let prompt = phase3_with_suggestions(stet, Some("remove x"));
        assert!(
            prompt.contains(stet),
            "stet output should appear verbatim"
        );
    }

    // -- Cross-phase delimiter isolation --

    #[test]
    fn normalize_plan_prompt_contains_instructions_and_delimiter_and_doc() {
        let doc = "Some PRD or notes.";
        let prompt = normalize_plan_prompt(doc);
        assert!(
            prompt.contains("canonical format"),
            "should describe canonical format: {prompt}"
        );
        assert!(
            prompt.contains("## Task 1"),
            "should describe task heading format: {prompt}"
        );
        assert!(
            prompt.contains("(parallel)"),
            "should mention parallel marker: {prompt}"
        );
        assert_eq!(
            prompt.matches(DOC_DELIMITER).count(),
            2,
            "exactly two ---DOC--- delimiters"
        );
        assert!(
            prompt.contains("Some PRD or notes."),
            "should contain document content verbatim"
        );
    }

    #[test]
    fn normalize_plan_prompt_fences_empty_doc() {
        let prompt = normalize_plan_prompt("");
        assert!(prompt.contains("---DOC---\n\n---DOC---"));
    }

    #[test]
    fn build_normalize_prompt_from_template_replaces_placeholder() {
        let template = "Convert the following:\n{{DOC}}\nEnd.";
        let doc = "my plan content";
        let prompt = build_normalize_prompt_from_template(template, doc);
        assert!(!prompt.contains("{{DOC}}"), "placeholder should be replaced");
        assert!(prompt.contains("my plan content"), "document content should appear");
        assert!(prompt.starts_with("Convert the following:\n"));
        assert!(prompt.ends_with("\nEnd."));
    }

    #[test]
    fn delimiters_are_distinct_across_phases() {
        let p1 = phase1("task");
        let p2 = phase2("plan");
        let p3 = phase3("stet");

        assert!(
            !p1.contains("---PLAN---"),
            "phase 1 must not use PLAN delimiter"
        );
        assert!(
            !p1.contains("---STET---"),
            "phase 1 must not use STET delimiter"
        );
        assert!(
            !p2.contains("---TASK---"),
            "phase 2 must not use TASK delimiter"
        );
        assert!(
            !p2.contains("---STET---"),
            "phase 2 must not use STET delimiter"
        );
        assert!(
            !p3.contains("---TASK---"),
            "phase 3 must not use TASK delimiter"
        );
        assert!(
            !p3.contains("---PLAN---"),
            "phase 3 must not use PLAN delimiter"
        );
    }
}
