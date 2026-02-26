# Stet review quality and triage

## Principle

Do not blindly address every stet finding. Stet reviews can include false positives, outdated suggestions, or suggestions that would make the code wrong. Triage first, then fix or dismiss.

## Workflow

1. **Run the review** — e.g. `stet run` or `stet commitmsg --commit-and-review`.
2. **Triage each finding** — For each finding, decide: **actionable** (fix the code) or **not actionable** (dismiss with reason).
3. **Fix only actionable findings** — Apply code changes only for findings that are real issues and where the suggested fix is correct.
4. **Dismiss the rest** — Run `stet dismiss <id> <reason>` for every non-actionable finding, using the correct reason so stet can improve (optimize/shadowing).

## Dismiss reasons

| Reason | Use when |
|--------|----------|
| **false_positive** | Not a real issue; model misread; redundant or low-signal nit. |
| **already_correct** | Code already correct; concern addressed; finding refers to something that already exists or was fixed. |
| **wrong_suggestion** | Suggestion is wrong or harmful (wrong API, would break or inconsistent code). |
| **out_of_scope** | Wrong scope (e.g. generated files, meta/curated docs). |

Quick pick: Worse/inconsistent → `wrong_suggestion`; wrong scope → `out_of_scope`; already correct → `already_correct`; else → `false_positive`.

## Examples

- **Finding:** "Missing field X in struct Y" — but the field exists and is initialized everywhere.  
  **Action:** Dismiss with **already_correct**.

- **Finding:** "Pass empty slice instead of None for extra_args" — the function takes `&[String]`; in Rust an empty slice is the correct way to pass "no args," not `Option`.  
  **Action:** Dismiss with **wrong_suggestion** (the suggested "fix" would be wrong).

- **Finding:** "Potential panic on unwrap_or_default" — but the code path is safe and `unwrap_or_default()` is used intentionally.  
  **Action:** Dismiss with **false_positive**.

- **Finding:** "Missing validation for extra_args before passing to run" — extra args are user-controlled config, same as other CLI args; no extra validation needed.  
  **Action:** Dismiss with **false_positive** or **wrong_suggestion** if the suggestion would add unnecessary code.
