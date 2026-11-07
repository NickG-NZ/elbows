#!/usr/bin/env bash
# One-time Doppler setup for elbows. Run by a human from a machine logged in with `doppler login`.
# Creates project `elbows` with a branch config `dev_agents`, which inherits the shared infra
# credentials in root `dev` plus agent-only settings, then mints a read-only service token for it.
# That token is the single value you paste into each Claude Code cloud environment as DOPPLER_TOKEN.
set -euo pipefail
PROJECT=elbows
CONFIG=dev_agents

doppler projects get "$PROJECT" >/dev/null 2>&1 || doppler projects create "$PROJECT" --description "elbows monorepo"
doppler configs get "$CONFIG" -p "$PROJECT" >/dev/null 2>&1 || doppler configs create "$CONFIG" -p "$PROJECT" -e dev

cat <<MSG
Shared infra credentials go in the root \`dev\` config; $CONFIG inherits them (never the held-out salt):
  doppler secrets set -p $PROJECT -c dev MODAL_TOKEN_ID MODAL_TOKEN_SECRET   # prompts; later HF_TOKEN, WANDB_API_KEY
Agent-only settings go in $CONFIG:
  doppler secrets set -p $PROJECT -c $CONFIG AR_LEDGER=modal
MSG
read -rp "Create the service token now? [y/N] " yn
if [[ "$yn" == [yY] ]]; then
  echo "DOPPLER_TOKEN (shown once; paste into the cloud environment's Environment variables):"
  doppler configs tokens create claude-cloud -p "$PROJECT" -c "$CONFIG" --plain
fi
