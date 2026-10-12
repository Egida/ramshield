#!/usr/bin/env python3
"""Run one prioritized BACKLOG task locally and publish it as a draft PR.

The timer is deliberately single-flight. An open task PR pauses the queue until
human review/merge; a failed run is recorded and also pauses the queue.
"""

from __future__ import annotations

import fcntl
import json
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
BASE_BRANCH = "audit-branch"
TASK_ID_RE = re.compile(r"(?i)(?:^|[^a-z0-9])a[-_]?([0-9]{3,4})(?:[^0-9]|$)")
TASK_ROW_RE = re.compile(r"^\|\s*A-(\d{3,4})\s*\|")
PRIORITY_RE = re.compile(r"^##\s+P([0-2])\b")
ACTIONABLE = {"OPEN", "PARTIAL"}


@dataclass(frozen=True)
class Task:
    task_id: str
    priority: int
    area: str
    description: str
    status: str


def run(args: list[str], *, cwd: Path = REPO, timeout: int = 90) -> str:
    result = subprocess.run(
        args,
        cwd=cwd,
        check=False,
        capture_output=True,
        text=True,
        timeout=timeout,
        env={**os.environ, "GH_PROMPT_DISABLED": "1"},
    )
    if result.returncode:
        raise RuntimeError(
            f"command failed ({result.returncode}): {args[0]} {args[1:]}\n"
            f"{result.stderr[-4000:]}"
        )
    return result.stdout


def parse_tasks(backlog: str) -> list[Task]:
    priority: int | None = None
    tasks: list[Task] = []
    for line in backlog.splitlines():
        heading = PRIORITY_RE.match(line.strip())
        if heading:
            priority = int(heading.group(1))
            continue
        match = TASK_ROW_RE.match(line.strip())
        if not match or priority is None:
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) < 4:
            continue
        status = re.sub(r"\s+", " ", cells[-1]).upper()
        tasks.append(
            Task(
                task_id=f"A-{int(match.group(1)):03d}",
                priority=priority,
                area=cells[1],
                description=cells[2],
                status=status,
            )
        )
    return tasks


def task_id_for_pr(pr: dict[str, object]) -> str | None:
    searchable = f"{pr.get('headRefName', '')} {pr.get('title', '')}"
    match = TASK_ID_RE.search(searchable)
    return f"A-{int(match.group(1)):03d}" if match else None


def select_task(backlog: str, prs: list[dict[str, object]]) -> tuple[Task | None, str]:
    tasks = parse_tasks(backlog)
    merged = {
        task_id_for_pr(pr)
        for pr in prs
        if pr.get("state") == "MERGED" and task_id_for_pr(pr)
    }
    open_task_prs = [
        pr for pr in prs if pr.get("state") == "OPEN" and task_id_for_pr(pr)
    ]
    if open_task_prs:
        pr = open_task_prs[0]
        return None, (
            f"waiting for human review/merge of {task_id_for_pr(pr)} PR "
            f"#{pr.get('number')}: {pr.get('url')}"
        )

    closed_unmerged = [
        pr
        for pr in prs
        if pr.get("state") == "CLOSED"
        and not pr.get("mergedAt")
        and task_id_for_pr(pr)
    ]
    if closed_unmerged:
        pr = closed_unmerged[0]
        return None, (
            f"manual resolution required for closed, unmerged "
            f"{task_id_for_pr(pr)} PR #{pr.get('number')}: {pr.get('url')}"
        )

    for task in sorted(tasks, key=lambda item: (item.priority, int(item.task_id[2:]))):
        if task.task_id in merged:
            if (
                task.status in ACTIONABLE
                or task.status.startswith("IN PROGRESS")
                or task.status.startswith("BLOCKED")
            ):
                return None, (
                    f"{task.task_id} has a merged task PR but backlog status is "
                    f"{task.status}; reconcile acceptance before continuing"
                )
            continue
        if task.status.startswith("IN PROGRESS") or task.status.startswith("BLOCKED"):
            return None, f"{task.task_id} is {task.status}; manual status/review required"
        if task.status in ACTIONABLE:
            return task, "selected by priority and backlog order"
    return None, "no OPEN or PARTIAL backlog rows remain"


def state_root() -> Path:
    base = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state"))
    return base / "ramshield-backlog-dispatcher"


def load_state(path: Path) -> dict[str, object]:
    if not path.exists():
        return {}
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
        return value if isinstance(value, dict) else {}
    except (OSError, json.JSONDecodeError):
        raise RuntimeError(f"dispatcher state is unreadable: {path}")


def save_state(path: Path, state: dict[str, object]) -> None:
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def log(message: str, *, logfile: Path | None = None) -> None:
    timestamp = datetime.now(timezone.utc).isoformat(timespec="seconds")
    line = f"{timestamp} {message}"
    print(line, flush=True)
    if logfile is not None:
        with logfile.open("a", encoding="utf-8") as stream:
            stream.write(line + "\n")


def backlog_for_base(base_sha: str) -> str:
    return run(["git", "show", f"{base_sha}:BACKLOG.md"])


def pull_requests() -> list[dict[str, object]]:
    raw = run(
        [
            "gh",
            "pr",
            "list",
            "--repo",
            "grep999/ramshield",
            "--state",
            "all",
            "--limit",
            "100",
            "--json",
            "number,title,headRefName,state,mergedAt,url",
        ]
    )
    value = json.loads(raw)
    if not isinstance(value, list):
        raise RuntimeError("GitHub PR response was not a list")
    return value


def worker_prompt(task: Task, base_sha: str, pr_number: str | None) -> str:
    release_gate = (
        f"The dispatcher implementation is PR #{pr_number}. Do not start work "
        "until that PR has merged."
        if pr_number
        else ""
    )
    return f"""You are the local Codex worker for RamShield backlog {task.task_id}.

Task selected from BACKLOG.md:
- Priority: P{task.priority}
- Area: {task.area}
- Acceptance: {task.description}
- Starting base SHA: {base_sha}

{release_gate}

Work only on this task. Read repository instructions and relevant source before
editing. Keep the change bounded and integrate with existing module boundaries;
do not add a framework or unrelated refactor. Run the relevant checks and record
the exact commands and results in your final response. Update BACKLOG.md only
with accurate task evidence; never claim hosted CI or human review that has not
passed. Do not commit, push, create issues/PRs, merge, deploy, or change
production settings; the dispatcher handles draft PR creation after validating
your working tree. Never modify kiddo_fix, PR #3, PR #5, or PR #19 unless the
selected task explicitly requires it (it should not). Stop and report blockers
instead of widening scope or weakening a gate.
"""


def create_task_pr(task: Task, branch: str) -> str:
    run(["git", "push", "--set-upstream", "origin", branch], cwd=REPO, timeout=180)
    title = f"[{task.task_id}] {task.area}"[:120]
    body = (
        f"## Backlog task {task.task_id}\n\n"
        f"Priority: P{task.priority}\n\n"
        f"{task.description}\n\n"
        "This draft was prepared by the local Codex backlog dispatcher. "
        "Review the exact candidate SHA and workflow results before accepting. "
        "The execution transcript is retained on the dispatch host.\n\n"
        "The dispatcher never merges or deploys."
    )
    result = subprocess.run(
        [
            "gh",
            "pr",
            "create",
            "--repo",
            "grep999/ramshield",
            "--draft",
            "--base",
            BASE_BRANCH,
            "--head",
            branch,
            "--title",
            title,
            "--body",
            body,
        ],
        cwd=REPO,
        check=False,
        capture_output=True,
        text=True,
        timeout=180,
        env={**os.environ, "GH_PROMPT_DISABLED": "1"},
    )
    if result.returncode:
        raise RuntimeError(f"draft PR creation failed: {result.stderr[-4000:]}")
    return result.stdout.strip()


def dispatch(state_dir: Path, release_pr: str | None, *, dry_run: bool) -> int:
    state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    lock_path = state_dir / "dispatcher.lock"
    with lock_path.open("w", encoding="utf-8") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            log("another dispatcher run holds the singleton lock")
            return 0

        if release_pr:
            release_state = json.loads(
                run(
                    [
                        "gh",
                        "pr",
                        "view",
                        release_pr,
                        "--repo",
                        "grep999/ramshield",
                        "--json",
                        "state",
                    ]
                )
            )["state"]
            if release_state != "MERGED":
                log(f"dispatcher release PR #{release_pr} is {release_state}; no work launched")
                return 0

        run(["git", "fetch", "origin", BASE_BRANCH], timeout=180)
        base_sha = run(["git", "rev-parse", f"origin/{BASE_BRANCH}"]).strip()
        prs = pull_requests()
        task, reason = select_task(backlog_for_base(base_sha), prs)
        if task is None:
            log(f"queue idle: {reason}")
            return 0
        log(f"{task.task_id} P{task.priority}: {reason}")
        if dry_run:
            return 0

        state_path = state_dir / "state.json"
        state = load_state(state_path)
        failures = state.setdefault("failed", {})
        if isinstance(failures, dict) and failures.get(task.task_id) == base_sha:
            log(f"queue paused: {task.task_id} already failed on base {base_sha}; inspect state/logs")
            return 1

        codex = shutil.which("codex")
        if not codex:
            raise RuntimeError("codex executable not found in PATH")
        run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        log_path = state_dir / "logs" / f"{task.task_id}-{run_id}.jsonl"
        log_path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        branch = f"codex/{task.task_id.lower()}-{run_id.lower()}"
        worktree = state_dir / "worktrees" / f"{task.task_id.lower()}-{run_id.lower()}"
        worktree.parent.mkdir(parents=True, exist_ok=True, mode=0o700)

        try:
            run(["git", "worktree", "add", "-b", branch, str(worktree), base_sha], timeout=180)
            prompt = worker_prompt(task, base_sha, release_pr)
            with log_path.open("w", encoding="utf-8") as output:
                result = subprocess.run(
                    [
                        codex,
                        "exec",
                        "--json",
                        "--approve-for-me",
                        "-C",
                        str(worktree),
                        prompt,
                    ],
                    cwd=worktree,
                    check=False,
                    stdout=output,
                    stderr=subprocess.STDOUT,
                    timeout=6500,
                    env={**os.environ, "GH_PROMPT_DISABLED": "1"},
                )
            if result.returncode:
                raise RuntimeError(f"Codex exited {result.returncode}; see {log_path}")
            head = run(["git", "rev-parse", "HEAD"], cwd=worktree).strip()
            if head != base_sha:
                raise RuntimeError(f"worker committed unexpectedly; inspect {worktree} ({head})")
            run(["git", "add", "--all"], cwd=worktree)
            staged = run(["git", "diff", "--cached", "--name-only"], cwd=worktree).splitlines()
            if not staged:
                raise RuntimeError(f"worker produced no changes; see {log_path}")
            run(["git", "diff", "--cached", "--check"], cwd=worktree)
            run(
                ["git", "commit", "-m", f"backlog({task.task_id.lower()}): implement prioritized task"],
                cwd=worktree,
                timeout=180,
            )
            url = create_task_pr(task, branch)
            state["last_success"] = {
                "task": task.task_id,
                "base_sha": base_sha,
                "head_sha": run(["git", "rev-parse", "HEAD"], cwd=worktree).strip(),
                "pr_url": url,
                "log": str(log_path),
                "finished_at": datetime.now(timezone.utc).isoformat(),
            }
            if isinstance(failures, dict):
                failures.pop(task.task_id, None)
            save_state(state_path, state)
            log(f"draft PR created for {task.task_id}: {url}; log: {log_path}")
            return 0
        except Exception as error:  # noqa: BLE001 - persist fail-closed state for the timer.
            failures[task.task_id] = base_sha
            state["last_failure"] = {
                "task": task.task_id,
                "base_sha": base_sha,
                "reason": str(error),
                "worktree": str(worktree),
                "log": str(log_path),
                "failed_at": datetime.now(timezone.utc).isoformat(),
            }
            save_state(state_path, state)
            log(f"queue paused after {task.task_id} failure: {error}")
            return 1


def main(argv: list[str]) -> int:
    dry_run = "--dry-run" in argv
    if any(arg not in {"--dry-run"} for arg in argv):
        print("usage: backlog_dispatcher.py [--dry-run]", file=sys.stderr)
        return 2
    release_pr = os.environ.get("RAMSHIELD_DISPATCHER_ENABLE_AFTER_PR")
    return dispatch(state_root(), release_pr, dry_run=dry_run)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
