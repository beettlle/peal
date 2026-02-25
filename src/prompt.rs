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

#[cfg(test)]
mod tests {
    use super::*;

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
}
