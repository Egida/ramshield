#!/bin/bash
# ansible-dev-cycle.sh — run the dev cycle as an ansible playbook
# Usage: ./ansible-dev-cycle.sh [tags] [project_dir]
# Skeleton: same for every project. Only vars/default.yml changes.
#
# Examples:
#   ./ansible-dev-cycle.sh                    # full loop
#   ./ansible-dev-cycle.sh concept            # just CONCEPT phase
#   ./ansible-dev-cycle.sh release quality    # release + CI check
#   ./ansible-dev-cycle.sh "" /other/repo     # run against different repo
#
set -euo pipefail

ANSIBLE_DIR="${2:-$(dirname "$0")}"
cd "$ANSIBLE_DIR"

export ANSIBLE_CONFIG=./ansible.cfg

if [ -z "$1" ]; then
  ansible-playbook -i inventory/hosts playbook.yml
else
  ansible-playbook -i inventory/hosts --tags "$1" playbook.yml
fi
