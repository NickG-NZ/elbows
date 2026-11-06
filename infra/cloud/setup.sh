#!/bin/bash
# Claude Code cloud environment SETUP SCRIPT. Paste into: claude.ai/code -> environment -> Setup script.
# Runs as root on Ubuntu 24.04 before Claude starts, and the result is cached (~7 days).
# Environment variables are NOT available here, so this script never needs secrets.
# Must exit 0 and finish in < ~5 min.

# Doppler CLI (secrets are fetched later by the SessionStart hook using $DOPPLER_TOKEN)
(curl -Ls --tlsv1.2 --proto "=https" --retry 3 https://cli.doppler.com/install.sh | sh) || echo "doppler install failed"

# Modal client for GPU experiments (also installed in the repo venv by `uv sync`)
uv tool install modal >/dev/null 2>&1 || true

exit 0
