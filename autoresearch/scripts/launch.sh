#!/usr/bin/env bash
# Launch autoresearch lanes in parallel, one Claude Code session per lane.
#
#   autoresearch/scripts/launch.sh [--local] <task> <lane>[:<focus>] [<lane>[:<focus>] ...]
#
#   autoresearch/scripts/launch.sh cartpole-swingup \
#     "cem:improve the CEM baseline (sampling, schedules, network)" \
#     "ppo:replace CEM with a fast batched PPO" \
#     "robust:robustness to physics randomisation" \
#     "free:anything; prioritise surprising ideas"
#
# Default: cloud sessions (`claude --cloud`) on your default cloud environment (choose it with
# /remote-env). They clone the GitHub remote at the current branch, so push first.
# --local: a git worktree + tmux pane per lane on this machine (fine for CPU tasks / debugging).
set -euo pipefail

mode=cloud
if [[ "${1:-}" == "--local" ]]; then mode=local; shift; fi
task="${1:?usage: launch.sh [--local] <task> <lane>[:<focus>] ...}"; shift
[[ $# -gt 0 ]] || { echo "give at least one lane" >&2; exit 2; }
root="$(git rev-parse --show-toplevel)"
[[ -f "$root/autoresearch/tasks/$task/task.toml" ]] || { echo "no such task: $task" >&2; exit 2; }

if [[ $mode == cloud ]]; then
  branch="$(git rev-parse --abbrev-ref HEAD)"
  if ! git diff --quiet HEAD || [[ -n "$(git log "origin/$branch..HEAD" 2>/dev/null)" ]]; then
    echo "warning: local changes or unpushed commits; cloud sessions clone origin/$branch" >&2
  fi
fi

for spec in "$@"; do
  lane="${spec%%:*}"
  focus=""; [[ "$spec" == *:* ]] && focus="${spec#*:}"
  prompt="You are an autoresearch lane. TASK=$task LANE=$lane FOCUS=${focus:-none}.
Read autoresearch/program.md and follow it exactly, starting with Setup. Run experiments with \`uv run ar run\`.
Never stop to ask whether to continue; loop until interrupted."
  if [[ $mode == cloud ]]; then
    echo ">> cloud lane $lane"
    claude --cloud "$prompt"
  else
    echo ">> local lane $lane (worktree ar-$task-$lane)"
    (cd "$root" && claude -w "ar-$task-$lane" --tmux --permission-mode auto "$prompt") &
    sleep 2
  fi
done
wait
