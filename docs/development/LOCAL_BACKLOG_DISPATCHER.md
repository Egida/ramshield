# Local backlog dispatcher

The user-level systemd timer checks `audit-branch` every 15 minutes and runs at most one prioritized backlog task at a time using the local Codex CLI. It selects the lowest-numbered priority first (P0, then P1, then P2), skips tasks with merged task PRs, and pauses whenever a task PR is open, a backlog row is blocked/in progress, or a prior run failed on the current base SHA.

Each run uses an isolated git worktree. The dispatcher validates the resulting diff, commits it, pushes a branch, and opens a **draft** PR. It never merges, deploys, or marks hosted checks or human review as passed. The next task waits until the draft is merged. Closed, unmerged task PRs require manual resolution.

The user service has a release guard: it does not launch a worker until the dispatcher implementation PR configured in `RAMSHIELD_DISPATCHER_ENABLE_AFTER_PR` is merged. Keep the local worktree and this checkout available after merging so the timer can continue to run. A powered-off or disconnected machine cannot dispatch; systemd catches up after boot.

## Install

The unit does not hard-code a checkout path. Point it at the checkout to work
from by writing one file:

```sh
mkdir -p ~/.config
printf 'RS_DISPATCHER_SCRIPT=%s/.github/scripts/backlog_dispatcher.py\n' \
  /path/to/ramshield > ~/.config/ramshield-backlog-dispatcher.env
install -Dm644 systemd/user/ramshield-backlog-dispatcher.service \
  ~/.config/systemd/user/ramshield-backlog-dispatcher.service
install -Dm644 systemd/user/ramshield-backlog-dispatcher.timer \
  ~/.config/systemd/user/ramshield-backlog-dispatcher.timer
systemctl --user daemon-reload
systemctl --user enable --now ramshield-backlog-dispatcher.timer
```

The dispatcher resolves its own repository from the script path, so the
script location is the only thing that has to be configured. If
`RS_DISPATCHER_SCRIPT` is unset, the unit fails with that message rather
than starting in the wrong directory.

## Operate

```sh
systemctl --user status ramshield-backlog-dispatcher.timer
systemctl --user start ramshield-backlog-dispatcher.service
journalctl --user -u ramshield-backlog-dispatcher.service
systemctl --user disable --now ramshield-backlog-dispatcher.timer
```

The singleton lock, state, and private execution transcripts live under `${XDG_STATE_HOME:-~/.local/state}/ramshield-backlog-dispatcher`. Inspect a failed run and resolve it before removing its task entry from `state.json`; failures pause that task on the same base SHA. The timer does not delete old worktrees or transcripts automatically.
