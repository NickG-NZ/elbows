#!/usr/bin/env bash
# One-time Modal setup for GPU experiments. Run by a human after `modal setup`.
# The held-out salt lives ONLY in this Modal secret: it reaches the eval step inside the GPU
# container, never the agent's environment, and it's never stored in Doppler.
set -euo pipefail
modal secret create autoresearch-holdout AR_HOLDOUT_SALT="$(openssl rand -hex 16)" --force
modal volume create autoresearch-cache 2>/dev/null || true
echo "Next: create a Modal token for agents (Modal dashboard -> Settings -> API tokens),"
echo "store it in Doppler (bootstrap_doppler.sh), and set a workspace spend limit in Modal."
