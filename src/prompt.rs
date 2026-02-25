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
/// The stet output is wrapped in `---STET---` delimiters.  This prompt is
/// used by SP-4.4 when stet reports findings after Phase 2 execution.
pub fn phase3(stet_output: &str) -> String {
    format!(
        "Address the following stet review findings. Apply fixes and run tests.\n\n\
         {STET_DELIMITER}\n\
         {stet_output}\n\
         {STET_DELIMITER}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Phase 1 tests --

    #[test]
    fn phase1_wraps_task_in_delimiters() {
        let prompt = phase1("Add a login form with OAuth support.");
        assert!(
            prompt.starts_with(
                "Create a plan for implementing this task:\n\n---TASK---\n"
            ),
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

    // -- Cross-phase delimiter isolation --

    #[test]
    fn delimiters_are_distinct_across_phases() {
        let p1 = phase1("task");
        let p2 = phase2("plan");
        let p3 = phase3("stet");

        assert!(!p1.contains("---PLAN---"), "phase 1 must not use PLAN delimiter");
        assert!(!p1.contains("---STET---"), "phase 1 must not use STET delimiter");
        assert!(!p2.contains("---TASK---"), "phase 2 must not use TASK delimiter");
        assert!(!p2.contains("---STET---"), "phase 2 must not use STET delimiter");
        assert!(!p3.contains("---TASK---"), "phase 3 must not use TASK delimiter");
        assert!(!p3.contains("---PLAN---"), "phase 3 must not use PLAN delimiter");
    }
}
