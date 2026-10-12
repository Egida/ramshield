import { readFile } from "node:fs/promises";
import { HandoffSchema } from "./handoff.schema.mjs";

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("Usage: node validate.mjs <handoff.json> [handoff.json ...]");
  process.exit(2);
}

let invalid = false;
for (const file of files) {
  let parsed;
  try {
    parsed = JSON.parse(await readFile(file, "utf8"));
  } catch (error) {
    console.error(`FAIL ${file}: cannot read valid JSON (${error.message})`);
    invalid = true;
    continue;
  }

  const result = HandoffSchema.safeParse(parsed);
  if (!result.success) {
    console.error(`FAIL ${file}:`);
    for (const issue of result.error.issues) {
      console.error(`  - ${issue.path.join(".") || "<root>"}: ${issue.message}`);
    }
    invalid = true;
    continue;
  }

  console.log(`PASS ${file}: schema v${result.data.schema_version}, task ${result.data.task_id}, status ${result.data.status}, head ${result.data.head_sha}`);
}
if (invalid) process.exit(1);
