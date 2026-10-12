# Agent-flow handoff contract

This directory defines the first, deliberately small contract for agent-to-agent handoffs. It is validation infrastructure, not a claim that an agent or recovery task passed.

## Contract

Every handoff carries a schema version, task ID, role, status, exact 40-character candidate SHA, summary, changed-file list, executed-check records, blockers, and timestamp.

Rules enforced by Zod:

- `pass` requires every listed check to have outcome `passed` and exit code `0`, with no blockers.
- `fail` requires at least one check with outcome `failed` and a non-zero exit code.
- `blocked` requires an explicit blocker.
- `skipped` and `not_run` are evidence of non-execution, never PASS.
- Unknown fields are rejected to prevent silent contract drift.

## Local validation

From this directory:

```sh
npm install --ignore-scripts --no-audit --no-fund
npm test
npm run validate
```

The workflow installs the exact Zod version declared in `package.json` and runs the same tests on the GitHub runner.

## Operating boundary

A handoff is trustworthy only when its SHA matches the candidate under review and its evidence points to real command or workflow output. Do not commit a handoff record to the same source commit it claims to validate: that changes the SHA. A later rollout can transport handoffs as workflow artifacts or comments and validate them against this schema before the coordinator consumes them.

This pilot does not grant merge/deploy authority, enable production mesh, or replace Rust CI, Backlog Gate, Review Pipeline, or independent review.
