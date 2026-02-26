use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::error::PealError;

/// Compiled once; the pattern is a valid literal so init cannot fail at runtime.
static HEADING_RE: OnceLock<Regex> = OnceLock::new();

fn heading_re() -> &'static Regex {
    HEADING_RE.get_or_init(|| {
        Regex::new(r"^## Task\s+(\d+)\s*(\(parallel\))?\s*$").expect("valid literal regex")
    })
}

/// A single task parsed from the plan file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub index: u32,
    pub content: String,
    pub parallel: bool,
}

/// An execution segment: either a sequential task or a block of parallel tasks.
///
/// Consecutive tasks with `parallel == true` form one parallel block.
/// A single parallel-marked task is treated as sequential (per PRD §14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Sequential(u32),
    Parallel(Vec<u32>),
}

/// The result of parsing a plan file: an ordered list of tasks and their
/// execution segments (sequential singletons and parallel blocks).
#[derive(Debug, Clone)]
pub struct ParsedPlan {
    pub tasks: Vec<Task>,
    pub segments: Vec<Segment>,
}

/// Read a plan file at `path` and parse it into tasks and segments.
///
/// Rejects non-UTF-8 content and I/O failures with `PealError::InvalidPlanFile`
/// or `PealError::PlanFileNotFound` when the file does not exist.
pub fn parse_plan_file(path: &Path) -> anyhow::Result<ParsedPlan> {
    let bytes = std::fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!(PealError::PlanFileNotFound {
                path: path.to_path_buf(),
            })
        } else {
            anyhow::anyhow!(PealError::InvalidPlanFile {
                path: path.to_path_buf(),
            })
            .context(e)
        }
    })?;

    let content = String::from_utf8(bytes).map_err(|e| {
        anyhow::anyhow!(PealError::InvalidPlanFile {
            path: path.to_path_buf(),
        })
        .context(e)
    })?;

    parse_plan(&content)
}

/// Parse plan content (already a valid UTF-8 string) into tasks and segments.
///
/// Heading pattern: `^## Task\s+(\d+)\s*(\(parallel\))?\s*$` (CRLF normalised to LF).
/// Task body runs from the line after the heading until the next heading or EOF.
/// Tasks are returned sorted by ascending index; gaps are allowed.
pub fn parse_plan(content: &str) -> anyhow::Result<ParsedPlan> {
    let content = content.replace("\r\n", "\n");
    let heading_re = heading_re();

    let mut tasks: Vec<Task> = Vec::new();
    let mut current_index: Option<u32> = None;
    let mut current_parallel = false;
    let mut body_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        if let Some(caps) = heading_re.captures(line) {
            if let Some(idx) = current_index {
                tasks.push(Task {
                    index: idx,
                    content: body_lines.join("\n").trim().to_owned(),
                    parallel: current_parallel,
                });
            }
            // Capture 1 is \d+ so parse cannot fail.
            current_index = Some(
                caps[1]
                    .parse::<u32>()
                    .expect("regex guarantees digit-only capture"),
            );
            current_parallel = caps.get(2).is_some();
            body_lines.clear();
        } else if current_index.is_some() {
            body_lines.push(line);
        }
    }

    if let Some(idx) = current_index {
        tasks.push(Task {
            index: idx,
            content: body_lines.join("\n").trim().to_owned(),
            parallel: current_parallel,
        });
    }

    tasks.sort_by_key(|t| t.index);

    let segments = compute_segments(&tasks);

    Ok(ParsedPlan { tasks, segments })
}

impl ParsedPlan {
    /// Execution schedule (SP-5.1): ordered segments defining run order and parallel blocks.
    /// Sequential segment = one task index; parallel block = set of task indices run together.
    pub fn execution_schedule(&self) -> &[Segment] {
        &self.segments
    }

    /// Look up a task by its index. O(n) scan, fine for typical plan sizes (<50 tasks).
    pub fn task_by_index(&self, index: u32) -> Option<&Task> {
        self.tasks.iter().find(|t| t.index == index)
    }

    /// Return a new plan containing only the task with the given index.
    ///
    /// Segments are recomputed from the filtered task list.
    pub fn filter_single_task(self, index: u32) -> Result<ParsedPlan, PealError> {
        let available: Vec<u32> = self.tasks.iter().map(|t| t.index).collect();
        let tasks: Vec<Task> = self
            .tasks
            .into_iter()
            .filter(|t| t.index == index)
            .collect();
        if tasks.is_empty() {
            return Err(PealError::TaskNotFound { index, available });
        }
        let segments = compute_segments(&tasks);
        Ok(ParsedPlan { tasks, segments })
    }

    /// Return a new plan containing the task at `index` and all subsequent tasks.
    ///
    /// "Subsequent" means tasks whose position in the sorted plan is at or after
    /// the target index. Segments are recomputed from the filtered task list.
    pub fn filter_from_task(self, index: u32) -> Result<ParsedPlan, PealError> {
        let available: Vec<u32> = self.tasks.iter().map(|t| t.index).collect();
        let pos = self.tasks.iter().position(|t| t.index == index);
        match pos {
            None => Err(PealError::TaskNotFound { index, available }),
            Some(start) => {
                let tasks: Vec<Task> = self.tasks.into_iter().skip(start).collect();
                let segments = compute_segments(&tasks);
                Ok(ParsedPlan { tasks, segments })
            }
        }
    }
}

/// Group an ordered task list into execution segments.
///
/// Consecutive tasks with `parallel == true` form one `Segment::Parallel` block
/// (unless only one task, which is treated as `Segment::Sequential`).
pub(crate) fn compute_segments(tasks: &[Task]) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut i = 0;

    while i < tasks.len() {
        if tasks[i].parallel {
            let mut block = vec![tasks[i].index];
            i += 1;
            while i < tasks.len() && tasks[i].parallel {
                block.push(tasks[i].index);
                i += 1;
            }
            if block.len() == 1 {
                segments.push(Segment::Sequential(block[0]));
            } else {
                segments.push(Segment::Parallel(block));
            }
        } else {
            segments.push(Segment::Sequential(tasks[i].index));
            i += 1;
        }
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn empty_content_produces_no_tasks() {
        let plan = parse_plan("").unwrap();
        assert!(plan.tasks.is_empty());
        assert!(plan.segments.is_empty());
    }

    #[test]
    fn single_task_parsed() {
        let plan = parse_plan("## Task 1\nImplement the widget.").unwrap();

        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].index, 1);
        assert_eq!(plan.tasks[0].content, "Implement the widget.");
        assert!(!plan.tasks[0].parallel);
    }

    #[test]
    fn multiple_sequential_tasks() {
        let input = "\
## Task 1
First task body.

## Task 2
Second task body.

## Task 3
Third task body.
";
        let plan = parse_plan(input).unwrap();

        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].index, 1);
        assert_eq!(plan.tasks[1].index, 2);
        assert_eq!(plan.tasks[2].index, 3);
        assert_eq!(plan.tasks[0].content, "First task body.");
        assert_eq!(plan.tasks[2].content, "Third task body.");
    }

    #[test]
    fn parallel_suffix_detected() {
        let input = "\
## Task 1
Sequential task.

## Task 2 (parallel)
Parallel A.

## Task 3 (parallel)
Parallel B.

## Task 4
Back to sequential.
";
        let plan = parse_plan(input).unwrap();

        assert_eq!(plan.tasks.len(), 4);
        assert!(!plan.tasks[0].parallel);
        assert!(plan.tasks[1].parallel);
        assert!(plan.tasks[2].parallel);
        assert!(!plan.tasks[3].parallel);
    }

    #[test]
    fn body_captured_until_next_heading() {
        let input = "\
## Task 1
Line one.
Line two.

Still task 1.

## Task 2
Task two body.
";
        let plan = parse_plan(input).unwrap();

        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(
            plan.tasks[0].content,
            "Line one.\nLine two.\n\nStill task 1."
        );
        assert_eq!(plan.tasks[1].content, "Task two body.");
    }

    #[test]
    fn body_captured_until_eof() {
        let input = "\
## Task 1
Only task.
Multiple lines.
No trailing heading.";
        let plan = parse_plan(input).unwrap();

        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(
            plan.tasks[0].content,
            "Only task.\nMultiple lines.\nNo trailing heading."
        );
    }

    #[test]
    fn crlf_normalised_to_lf() {
        let input = "## Task 1\r\nDo something.\r\n\r\n## Task 2\r\nDo another.\r\n";
        let plan = parse_plan(input).unwrap();

        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].content, "Do something.");
        assert_eq!(plan.tasks[1].content, "Do another.");
    }

    #[test]
    fn gaps_in_indices_preserved_in_order() {
        let input = "\
## Task 5
Fifth.

## Task 1
First.

## Task 3
Third.
";
        let plan = parse_plan(input).unwrap();

        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].index, 1);
        assert_eq!(plan.tasks[1].index, 3);
        assert_eq!(plan.tasks[2].index, 5);
    }

    #[test]
    fn preamble_before_first_heading_ignored() {
        let input = "\
# My Plan

Some introductory text.

## Task 1
The actual task.
";
        let plan = parse_plan(input).unwrap();

        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].content, "The actual task.");
    }

    #[test]
    fn heading_with_extra_whitespace() {
        let input = "## Task   42   (parallel)   \nBody text.";
        let plan = parse_plan(input).unwrap();

        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].index, 42);
        assert!(plan.tasks[0].parallel);
        assert_eq!(plan.tasks[0].content, "Body text.");
    }

    #[test]
    fn non_task_headings_not_matched() {
        let input = "\
## Task 1
Body.

## Not a task heading
This should be part of task 1's body.
";
        // "## Not a task heading" does not match the regex, so it becomes
        // part of task 1's body.
        let plan = parse_plan(input).unwrap();

        assert_eq!(plan.tasks.len(), 1);
        assert!(plan.tasks[0].content.contains("## Not a task heading"));
    }

    // -- Segment / parallel-block tests --

    #[test]
    fn all_sequential_produces_sequential_segments() {
        let input = "## Task 1\nA\n\n## Task 2\nB\n\n## Task 3\nC\n";
        let plan = parse_plan(input).unwrap();

        assert_eq!(
            plan.segments,
            vec![
                Segment::Sequential(1),
                Segment::Sequential(2),
                Segment::Sequential(3),
            ]
        );
    }

    #[test]
    fn consecutive_parallel_tasks_form_block() {
        let input = "\
## Task 1
A

## Task 2 (parallel)
B

## Task 3 (parallel)
C

## Task 4
D
";
        let plan = parse_plan(input).unwrap();

        assert_eq!(
            plan.segments,
            vec![
                Segment::Sequential(1),
                Segment::Parallel(vec![2, 3]),
                Segment::Sequential(4),
            ]
        );
    }

    #[test]
    fn single_parallel_task_treated_as_sequential() {
        let input = "\
## Task 1
A

## Task 2 (parallel)
B

## Task 3
C
";
        let plan = parse_plan(input).unwrap();

        assert_eq!(
            plan.segments,
            vec![
                Segment::Sequential(1),
                Segment::Sequential(2),
                Segment::Sequential(3),
            ]
        );
    }

    #[test]
    fn multiple_parallel_blocks() {
        let input = "\
## Task 1
A

## Task 2 (parallel)
B

## Task 3 (parallel)
C

## Task 4
D

## Task 5 (parallel)
E

## Task 6 (parallel)
F

## Task 7 (parallel)
G
";
        let plan = parse_plan(input).unwrap();

        assert_eq!(
            plan.segments,
            vec![
                Segment::Sequential(1),
                Segment::Parallel(vec![2, 3]),
                Segment::Sequential(4),
                Segment::Parallel(vec![5, 6, 7]),
            ]
        );
        // SP-5.1: execution_schedule() is the single source for run order.
        assert_eq!(plan.execution_schedule(), plan.segments.as_slice());
    }

    // -- File-level tests --

    #[test]
    fn parse_plan_file_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.md");
        std::fs::write(&path, "## Task 1\nDo it.\n").unwrap();

        let plan = parse_plan_file(&path).unwrap();
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].content, "Do it.");
    }

    #[test]
    fn parse_plan_file_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.md");

        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&[0xFF, 0xFE, 0x80, 0x81]).unwrap();
        drop(file);

        let err = parse_plan_file(&path).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Invalid or missing plan file") || msg.contains("invalid utf-8"),
            "expected UTF-8 rejection, got: {msg}"
        );
    }

    #[test]
    fn parse_plan_file_not_found() {
        let err = parse_plan_file(Path::new("/no/such/file.md")).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("does not exist"),
            "expected file-not-found error, got: {msg}"
        );
    }

    // -- filter_single_task / filter_from_task tests --

    fn make_plan_123() -> ParsedPlan {
        parse_plan("## Task 1\nA\n\n## Task 2\nB\n\n## Task 3\nC\n").unwrap()
    }

    #[test]
    fn filter_single_task_returns_matching_task() {
        let plan = make_plan_123().filter_single_task(2).unwrap();
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].index, 2);
        assert_eq!(plan.segments, vec![Segment::Sequential(2)]);
    }

    #[test]
    fn filter_single_task_not_found() {
        let err = make_plan_123().filter_single_task(99).unwrap_err();
        match err {
            PealError::TaskNotFound { index, available } => {
                assert_eq!(index, 99);
                assert_eq!(available, vec![1, 2, 3]);
            }
            other => panic!("expected TaskNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn filter_from_task_returns_tail() {
        let plan = make_plan_123().filter_from_task(2).unwrap();
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].index, 2);
        assert_eq!(plan.tasks[1].index, 3);
        assert_eq!(
            plan.segments,
            vec![Segment::Sequential(2), Segment::Sequential(3)]
        );
    }

    #[test]
    fn filter_from_task_not_found() {
        let err = make_plan_123().filter_from_task(99).unwrap_err();
        match err {
            PealError::TaskNotFound { index, available } => {
                assert_eq!(index, 99);
                assert_eq!(available, vec![1, 2, 3]);
            }
            other => panic!("expected TaskNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn filter_from_last_task_returns_single() {
        let plan = make_plan_123().filter_from_task(3).unwrap();
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].index, 3);
    }

    #[test]
    fn filter_from_task_recomputes_parallel_segments() {
        let input = "\
## Task 1
A

## Task 2 (parallel)
B

## Task 3 (parallel)
C

## Task 4
D
";
        let plan = parse_plan(input).unwrap();
        assert_eq!(
            plan.segments,
            vec![
                Segment::Sequential(1),
                Segment::Parallel(vec![2, 3]),
                Segment::Sequential(4),
            ]
        );

        // Filtering from task 3 breaks the parallel block: task 3 alone becomes sequential.
        let filtered = plan.filter_from_task(3).unwrap();
        assert_eq!(filtered.tasks.len(), 2);
        assert_eq!(
            filtered.segments,
            vec![Segment::Sequential(3), Segment::Sequential(4)]
        );
    }

    /// Parses docs/plan-phase4.md, plan-phase5.md, plan-phase6.md, plan-phase7.md
    /// and asserts expected task counts and non-empty bodies.
    #[test]
    fn parse_docs_phase4_phase5_phase6_plan_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let plan4 = parse_plan_file(&root.join("docs/plan-phase4.md")).unwrap();
        let plan5 = parse_plan_file(&root.join("docs/plan-phase5.md")).unwrap();
        let plan6 = parse_plan_file(&root.join("docs/plan-phase6.md")).unwrap();
        let plan7 = parse_plan_file(&root.join("docs/plan-phase7.md")).unwrap();
        assert_eq!(plan4.tasks.len(), 7, "plan-phase4.md should have 7 tasks");
        assert_eq!(plan5.tasks.len(), 5, "plan-phase5.md should have 5 tasks");
        assert_eq!(plan6.tasks.len(), 3, "plan-phase6.md should have 3 tasks");
        assert_eq!(plan7.tasks.len(), 4, "plan-phase7.md should have 4 tasks");
        for t in &plan4.tasks {
            assert!(
                !t.content.trim().is_empty(),
                "phase4 task {} has non-empty body",
                t.index
            );
        }
        for t in &plan5.tasks {
            assert!(
                !t.content.trim().is_empty(),
                "phase5 task {} has non-empty body",
                t.index
            );
        }
        for t in &plan6.tasks {
            assert!(
                !t.content.trim().is_empty(),
                "phase6 task {} has non-empty body",
                t.index
            );
        }
        for t in &plan7.tasks {
            assert!(
                !t.content.trim().is_empty(),
                "phase7 task {} has non-empty body",
                t.index
            );
        }
    }

    // -- task_by_index tests --

    #[test]
    fn task_by_index_finds_existing() {
        let plan = make_plan_123();
        let t = plan.task_by_index(2).expect("task 2 should exist");
        assert_eq!(t.index, 2);
        assert_eq!(t.content, "B");
    }

    #[test]
    fn task_by_index_returns_none_for_missing() {
        let plan = make_plan_123();
        assert!(plan.task_by_index(99).is_none());
    }

    #[test]
    fn task_by_index_boundary_first_and_last() {
        let plan = make_plan_123();
        assert_eq!(plan.task_by_index(1).unwrap().index, 1);
        assert_eq!(plan.task_by_index(3).unwrap().index, 3);
    }

    #[test]
    fn task_by_index_zero_returns_none() {
        let plan = make_plan_123();
        assert!(plan.task_by_index(0).is_none());
    }
}
