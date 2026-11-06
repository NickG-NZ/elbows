#!/usr/bin/env bash
# SessionStart hook (wired in .claude/settings.json). Runs on every session start/resume.
#  * Cloud sessions: pull secrets from Doppler with the scoped service token in $DOPPLER_TOKEN and
#    export them for every later Bash command via $CLAUDE_ENV_FILE. Values are never printed.
#  * Local sessions: no-op unless ELBOWS_LOAD_SECRETS=1 (locally prefer `doppler run -- claude`).
set -uo pipefail
cd "${CLAUDE_PROJECT_DIR:-.}"

if [[ "${CLAUDE_CODE_REMOTE:-}" == "true" || "${ELBOWS_LOAD_SECRETS:-}" == "1" ]]; then
  if [[ -z "${DOPPLER_TOKEN:-}" ]]; then
    echo "elbows: DOPPLER_TOKEN not set in this environment; no secrets loaded"
  elif ! command -v doppler >/dev/null; then
    echo "elbows: doppler CLI missing (add infra/cloud/setup.sh as the environment setup script)"
  elif [[ -z "${CLAUDE_ENV_FILE:-}" ]]; then
    echo "elbows: CLAUDE_ENV_FILE not provided; cannot export secrets"
  else
    tmp="$(mktemp)"
    if doppler secrets download --no-file --format env >"$tmp" 2>/dev/null; then
      grep -v '^DOPPLER_' "$tmp" | sed 's/^/export /' >>"$CLAUDE_ENV_FILE"
      echo "elbows: loaded $(grep -vc '^DOPPLER_' "$tmp") secrets from Doppler"
    else
      echo "elbows: doppler secrets download failed (token revoked? *.doppler.com not in allowed domains?)"
    fi
    rm -f "$tmp"
  fi
  uv sync -q >/dev/null 2>&1 || echo "elbows: uv sync failed"
fi
exit 0
