//! The ansible quality gate must fail when CI failed.
//!
//! The role polls each workflow for the pushed SHA and exited 0 as soon as
//! a run was *completed* - whatever its conclusion. `ci_failed` was then
//! derived from the non-zero exit code, so a red CI run was reported as a
//! pass and the failure triage never ran.
//!
//! This runs the role's own shell block against a stubbed `gh`, so the
//! check is the gate and not a copy of it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The shell block of the "Wait for CI workflow to complete" task.
fn polling_block() -> String {
    let role = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ansible/roles/quality/tasks/main.yml"),
    )
    .expect("the quality role must be readable");

    let task = role
        .split("- name: Wait for CI workflow to complete on pushed SHA")
        .nth(1)
        .expect("the polling task must exist");
    let body = task
        .split("  shell: |")
        .nth(1)
        .expect("the polling task must have a shell block");
    let mut lines = Vec::new();
    for line in body.lines() {
        // The block's content is indented four spaces; the task's own keys
        // are indented two.
        let indent = line.len() - line.trim_start().len();
        if line.trim().is_empty() {
            lines.push("");
            continue;
        }
        if indent < 4 {
            break;
        }
        lines.push(&line[4..]);
    }
    assert!(!lines.is_empty(), "the polling block is empty");
    lines.join("\n")
}

/// Run the block with a `gh` that reports the given conclusion.
fn run_with_conclusion(conclusion: &str) -> i32 {
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "ramshield-quality-gate-{}-{}",
        std::process::id(),
        conclusion
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let sha = "0123456789abcdef0123456789abcdef01234567";
    // The stub answers every `gh run list` with one completed run.
    std::fs::write(
        dir.join("gh"),
        format!(
            "#!/bin/sh\ncat <<'JSON'\n[{{\"headSha\":\"{sha}\",\"conclusion\":\"{conclusion}\",\"status\":\"completed\"}}]\nJSON\n"
        ),
    )
    .expect("the gh stub must be writable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("gh"), std::fs::Permissions::from_mode(0o755))
            .expect("the gh stub must be executable");
    }

    let script = polling_block()
        .replace("{{ project_dir }}", dir.to_str().unwrap())
        .replace("{{ branch }}", "master")
        .replace("{{ ci_poll_max_seconds }}", "5")
        .replace("{{ ci_poll_interval_seconds }}", "1")
        .replace("{{ pushed_sha.stdout }}", sha)
        .replace("{{ item }}", "CI");
    std::fs::write(dir.join("gate.sh"), script).expect("the script must be writable");

    let path = format!(
        "{}:{}",
        dir.to_str().unwrap(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new("bash")
        .arg(dir.join("gate.sh"))
        .env("PATH", path)
        .output()
        .expect("the gate script must run");
    let _ = std::fs::remove_dir_all(&dir);
    out.status.code().unwrap_or(-1)
}

#[test]
fn the_quality_gate_passes_a_successful_workflow() {
    assert_eq!(
        run_with_conclusion("success"),
        0,
        "a successful workflow must pass the quality gate"
    );
}

#[test]
fn the_quality_gate_fails_a_failed_workflow() {
    assert_eq!(
        run_with_conclusion("failure"),
        1,
        "a failed workflow must fail the quality gate, not pass it"
    );
}

#[test]
fn the_quality_gate_fails_a_cancelled_workflow() {
    assert_eq!(
        run_with_conclusion("cancelled"),
        1,
        "a cancelled workflow must fail the quality gate"
    );
}

/// The role must not read the conclusion without checking it.
#[test]
fn the_quality_gate_compares_the_conclusion() {
    let block = polling_block();
    assert!(
        block.contains("\"success\""),
        "the polling block must compare the run's conclusion with success"
    );
    let _ = Path::new("/");
}
