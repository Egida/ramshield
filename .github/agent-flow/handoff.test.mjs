import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { HandoffSchema } from "./handoff.schema.mjs";

const fixture = JSON.parse(await readFile(new URL("./handoff.example.json", import.meta.url), "utf8"));

test("accepts the documented blocked handoff fixture", () => {
  assert.equal(HandoffSchema.safeParse(fixture).success, true);
});

test("rejects malformed candidate SHA", () => {
  assert.equal(HandoffSchema.safeParse({ ...fixture, head_sha: "abc123" }).success, false);
});

test("rejects PASS when a check was skipped or not run", () => {
  const result = HandoffSchema.safeParse({ ...fixture, status: "pass", blockers: [], checks: [{ name: "CI", outcome: "not_run" }] });
  assert.equal(result.success, false);
});

test("rejects FAIL without an observed failing check", () => {
  const result = HandoffSchema.safeParse({ ...fixture, status: "fail", blockers: [], checks: [{ name: "CI", outcome: "passed", exit_code: 0 }] });
  assert.equal(result.success, false);
});

test("rejects BLOCKED without a concrete blocker", () => {
  assert.equal(HandoffSchema.safeParse({ ...fixture, blockers: [] }).success, false);
});

test("rejects passed checks without exit_code zero", () => {
  const result = HandoffSchema.safeParse({ ...fixture, checks: [{ name: "CI", outcome: "passed" }] });
  assert.equal(result.success, false);
});
