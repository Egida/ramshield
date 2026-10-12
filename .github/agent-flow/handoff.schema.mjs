import { z } from "zod";

const CheckSchema = z.object({
  name: z.string().trim().min(1).max(120),
  outcome: z.enum(["passed", "failed", "skipped", "not_run"]),
  command: z.string().max(500).optional(),
  exit_code: z.number().int().optional(),
  evidence_url: z.string().url().optional(),
}).strict().superRefine((check, ctx) => {
  if (check.outcome === "passed" && check.exit_code !== 0) {
    ctx.addIssue({ code: "custom", path: ["exit_code"], message: "passed checks must record exit_code: 0" });
  }
  if (check.outcome === "failed" && (check.exit_code === undefined || check.exit_code === 0)) {
    ctx.addIssue({ code: "custom", path: ["exit_code"], message: "failed checks must record a non-zero exit_code" });
  }
  if ((check.outcome === "skipped" || check.outcome === "not_run") && check.exit_code !== undefined) {
    ctx.addIssue({ code: "custom", path: ["exit_code"], message: "skipped/not_run checks must not claim an exit code" });
  }
});

export const HandoffSchema = z.object({
  schema_version: z.literal(1),
  task_id: z.string().trim().min(1).max(80).regex(/^[A-Z0-9][A-Z0-9._-]*$/),
  role: z.enum(["architect", "coder", "tester", "reviewer", "coordinator"]),
  status: z.enum(["pass", "fail", "blocked"]),
  head_sha: z.string().regex(/^[0-9a-f]{40}$/),
  summary: z.string().trim().min(1).max(2000),
  changed_files: z.array(z.string().trim().min(1).max(500)).max(200),
  checks: z.array(CheckSchema).min(1).max(100),
  blockers: z.array(z.string().trim().min(1).max(1000)).max(50),
  created_at: z.string().datetime({ offset: true }),
}).strict().superRefine((handoff, ctx) => {
  if (handoff.status === "pass") {
    if (handoff.blockers.length > 0) {
      ctx.addIssue({ code: "custom", path: ["blockers"], message: "pass handoffs cannot contain blockers" });
    }
    if (handoff.checks.some((check) => check.outcome !== "passed")) {
      ctx.addIssue({ code: "custom", path: ["checks"], message: "pass requires every check to be passed" });
    }
  }
  if (handoff.status === "fail" && !handoff.checks.some((check) => check.outcome === "failed")) {
    ctx.addIssue({ code: "custom", path: ["checks"], message: "fail requires at least one failed check with a recorded exit code" });
  }
  if (handoff.status === "blocked" && handoff.blockers.length === 0) {
    ctx.addIssue({ code: "custom", path: ["blockers"], message: "blocked handoffs must explain at least one blocker" });
  }
});
