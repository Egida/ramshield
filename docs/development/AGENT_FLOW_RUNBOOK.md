# RamShield Agent Flow Runbook

## Goal

Make repository work execute as a verifiable pipeline rather than a sequence of untracked prompts. GitHub Issues are the durable task queue; pull requests and commits are the work products; GitHub Actions is the execution evidence; MCP coordinates discovery and routing; coding agents are replaceable workers.

## Control-plane model

```text
MCP coordinator
  -> inspect live issue/PR + exact SHA
  -> create bounded task with acceptance criteria and file ownership
  -> delegate to GitHub Copilot cloud agent (when enabled) or a named worker
  -> worker branch / draft PR
  -> GitHub Actions: tests, lint, build, artifact
  -> strict handoff contract tied to candidate SHA
  -> independent read-only reviewer
       -> FAIL/BLOCKED: root-cause task + bounded retry
       -> PASS: close dependency and dispatch next eligible task
```

MCP must not pretend to be the worker. A dispatch is not completion. A created PR is not acceptance. A successful schema test is not a Rust or recovery pass.

## Task state machine

`READY -> ASSIGNED -> WORKING -> VERIFYING -> REVIEWING -> ACCEPTED`

Failure transitions:
- `WORKING/VERIFYING -> BLOCKED` when execution evidence is missing or the worker cannot proceed.
- `VERIFYING -> REWORK` on an observed failure with logs.
- `REVIEWING -> REWORK` when the reviewer identifies a correctness defect.
- `REWORK -> ASSIGNED` only with a new bounded task, owner, and explicit regression target.
- `ACCEPTED` requires same-SHA worker handoff, exact-SHA required workflow results, and independent reviewer PASS.

Do not auto-loop indefinitely. Each task has a retry budget; after two failed attempts, create a root-cause/research task and stop automatic retries for that task until its cause and mitigation are recorded.

## Handoff contract

Every worker handoff must include:
- `schema_version`, task ID, role, status, exact 40-character candidate head SHA
- base SHA, changed paths, summary, known risks
- every command/check, outcome, exit code, and workflow/run URL
- blockers and next action
- timestamp with timezone

Rules:
- PASS requires all required checks explicitly passed with exit code 0, no blockers, and candidate SHA equal to the reviewed PR head.
- FAIL requires an observed failing check and non-zero exit code.
- BLOCKED requires a concrete blocker.
- Skipped, not run, inaccessible, stale SHA, synthetic PR merge SHA, or missing run is never PASS.
- The example fixture in `.github/agent-flow/handoff.example.json` is a BLOCKED fixture, not execution evidence.

## Ownership and parallelism

- Protocol coder: `crates/ramshield-mesh/src/transport.rs`, `crates/ramshield-enforcement/src/service/mesh.rs`.
- Test coder: `crates/ramshield-mesh/tests/mesh_acceptance.rs`, `scripts/test-mesh-recovery-sandbox.sh`, agreed test-only support.
- Independent reviewer: read-only, no code writes.
- Coordinator: tasks, dependencies, exact-SHA evidence and routing; no implementation writes.
- Do not allow concurrent agents to write the same path. Parallelize only disjoint files.

## GitHub-side independent workers

GitHub Copilot cloud agent can be assigned to a well-scoped issue when enabled for the repository. It works asynchronously in a GitHub-hosted environment, can run tests, and creates a pull request for review. Use it as an independent worker, not as the reviewer for its own change.

Supported launch paths include issue assignment and mentioning `@copilot` in an open pull request comment. For an issue assignment, select the target repository and base branch explicitly. For a PR comment, explicitly request a separate PR based on the safe target branch; Copilot otherwise may push directly to the existing PR branch. Verify dispatch by observing the Copilot 👀 reaction/session timeline and then a resulting branch/PR. A posted mention alone is not proof that an agent started. Availability depends on the Copilot plan and repository policy.

Give each task:
1. one bounded task and target area,
2. explicit acceptance criteria and forbidden paths,
3. requirement to run tests and report actual commands/results,
4. no permission to merge/deploy or alter production settings.

For PR #19, a Copilot worker must target a dedicated branch/PR based on `audit-branch` or the isolated PR #19 branch, depending on the task; never default to `master`. For agent-flow follow-up issue #21, the safe base is `automation/github-mcp-flow-pilot-20261012`. The current MCP connector can post comments and assign ordinary users but does not expose the Copilot-specific assignment operation with an explicit base-branch argument. Do not attempt an ordinary API assignment that might default to `master`. If the GitHub UI cannot select the exact safe branch, stop rather than dispatching to an unsafe base. If a Copilot mention receives no reaction/session, report NOT DISPATCHED and verify repository/plan enablement rather than claiming a worker is active.

## Workflow trust boundary

The current PR #20 workflow validates the handoff schema and publishes an artifact with candidate SHA and run URL. It is not a privileged dispatcher. A `workflow_run` coordinator only triggers when its workflow file exists on the default branch, and it runs with the default-branch workflow context. Therefore, do not add a write-token dispatcher to this draft branch and assume it will run safely.

Rollout sequence:
1. Keep the current PR validation workflow read-only (`contents: read`).
2. Prove exact-SHA artifact metadata and handoff schema.
3. Implement a dry-run coordinator that reads results and emits a proposed next action without writes.
4. Install trusted dispatch logic only through an explicitly reviewed default-branch change; separate untrusted PR validation from privileged event handling.
5. Validate source workflow identity, repository, branch, head SHA, artifact identity, and event/run ID; deduplicate by run ID + attempt + candidate SHA.
6. Restrict dispatch to an allowlist of task templates and safe branches; never accept arbitrary commands, branch names, paths, or shell fragments from artifacts.
7. Add bounded retries, a stop condition, and audit comments. Never merge or deploy automatically.

## First live workload: PR #19

Current known blocker: the multi-frame TCP acceptance test previously ACKed frames without applying their deltas. A correction was committed at `8bafe4836271f678aa5209801e3942617f117256` to merge frame deltas into receiver state before ACK. This change is not yet verified by exact-head CI; do not call it passing.

Next gates:
1. Run focused multi-frame acceptance test and relevant mesh/enforcement tests on the exact head.
2. Implement and verify receiver-confirmed completion proof bound to generation + recovery identity + captured WAL boundary + deterministic digest + complete frame set.
3. Add restart/loss/reorder/duplicate/concurrent-tail convergence tests.
4. Obtain protocol coder and test coder handoffs for the same exact SHA.
5. Independent reviewer checks same SHA and actual CI logs.
6. Keep PR Draft and production mesh disabled until all gates pass.

## Operational reporting

Every run report says:
- current PR head SHA,
- task ID and state,
- commits made,
- exact workflow run URLs and per-job status,
- what was executed vs only inspected,
- blocker and next owner/action.

No generic "working" updates. No claiming a run passed before GitHub reports completion. No merge, deploy, force-push, or rebase.
