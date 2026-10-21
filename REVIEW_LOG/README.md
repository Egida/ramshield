# Independent Review Log

This directory is the durable handoff point for independent commit reviews and for recording commits made on the review branch.

## Agent protocol

1. Read this file first.
2. Review entries under `commits/` by full commit SHA. Each entry must distinguish verified findings from hypotheses and identify the exact tree reviewed.
3. Do not treat commit messages, green CI, or a report generated from file metadata as a semantic code review.
4. Verify each finding against the exact commit tree and relevant surrounding code. Record file paths and line numbers where possible.
5. Check CI status for the exact SHA. State explicitly which checks were inspected and which were not run or unavailable.
6. Turn confirmed issues into bounded tasks with acceptance tests and disallowed scope. Treat unconfirmed concerns as investigation items.
7. Avoid duplicate work: search for an existing entry for the full SHA before adding one. Append a new entry rather than silently rewriting history.
8. Log every commit made on this review branch in `BRANCH_COMMITS.md`, including its SHA, parent/base, purpose, files changed, and validation performed.

## Status vocabulary

- **CONFIRMED** — reproduced or directly established from the reviewed source/evidence.
- **RISK / NEEDS VERIFICATION** — plausible concern not yet established.
- **NOT VERIFIED** — evidence was unavailable or the check was not performed.
- **NO FINDING** — a defined review was performed and no actionable defect was found within its stated scope.

## Scope and safety

This branch is a review/documentation handoff. Do not modify product code, merge, deploy, or imply production readiness from this log. Review the full SHA, not a moving branch name. Any finding must name the evidence and limitations.

## Current review target

- Repository: https://github.com/grep999/ramshield
- Master SHA at branch creation: `bd417cd9c79492dbe4a75de08186c9bf79f4e759`
- Commit: https://github.com/grep999/ramshield/commit/bd417cd9c79492dbe4a75de08186c9bf79f4e759
- This directory's first commit is documentation only; it is not itself a semantic review of the product code.

## Hourly collection

`.github/workflows/independent-review-log.yml` runs on the hour and reads
the remarks from the agent handoff issue (#31) with
`scripts/independent_review_log.py`. It writes:

- `REVIEW_LOG/commits/<sha>.md` - one file per reviewed commit. **A file
  that already exists is never rewritten.** A review entry is work, and
  the collector does not know what a better review would say.
- `REVIEW_LOG/INDEX.md` - a table of every review seen. This is the only
  file the collector rewrites; it is a view, not a review.
- `REVIEW_LOG/BRANCH_COMMITS.md` - the commits made on this branch.
  Entries are appended, never rewritten.

A remark counts as a review when it starts with
`## Independent master review - <sha>`. Anything else in the issue is
ignored. The collector records what the remark said; it does not
establish, upgrade or contradict a status. A status the remark does not
state is recorded as `NOT VERIFIED`.

The collector's own contract is `scripts/test_independent_review_log.py`.
It runs in the workflow and locally.

The schedule only fires for a workflow file on `master`, which is where
GitHub runs scheduled workflows from. The log itself is committed to
this branch, so `master` never carries it.
