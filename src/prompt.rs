//! Centralized prompt construction for all PEAL phases.
//!
//! Every prompt sent to the Cursor CLI is built here so there is exactly
//! one place to review, update, and test the strings.

/// Build the Phase 1 (plan) prompt for a given task.
///
/// Template: `"Create a plan for implementing this task: {task_content}"`
pub fn phase1(task_content: &str) -> String {
    format!("Create a plan for implementing this task: {task_content}")
}

/// Delimiter used to fence plan text inside the Phase 2 prompt, preventing
/// the agent from treating plan content as top-level instructions.
const PLAN_DELIMITER: &str = "---PLAN---";

/// Build the Phase 2 (execute) prompt for a given plan text.
///
/// The plan text is wrapped in `---PLAN---` delimiters so the agent can
/// distinguish the plan content from the instruction envelope.
pub fn phase2(plan_text: &str) -> String {
    format!(
        "Execute the following plan. Do not re-plan; only implement and test.\n\n\
         {PLAN_DELIMITER}\n\
         {plan_text}\n\
         {PLAN_DELIMITER}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Phase 1 tests --

    #[test]
    fn phase1_includes_task_content() {
        let prompt = phase1("Add a login form with OAuth support.");
        assert_eq!(
            prompt,
            "Create a plan for implementing this task: Add a login form with OAuth support."
        );
    }

    #[test]
    fn phase1_handles_multiline_content() {
        let content = "Step 1: do X.\nStep 2: do Y.\nStep 3: verify.";
        let prompt = phase1(content);
        assert!(prompt.starts_with("Create a plan for implementing this task: "));
        assert!(prompt.contains("Step 1: do X.\nStep 2: do Y.\nStep 3: verify."));
    }

    #[test]
    fn phase1_handles_empty_content() {
        let prompt = phase1("");
        assert_eq!(prompt, "Create a plan for implementing this task: ");
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
}
