# elbows

Robotics monorepo. Many coding agents (Claude Code, Codex, …) work here in parallel, locally and in the cloud.
This file is the shared agent guide; `CLAUDE.md` is a symlink to it, so edit `AGENTS.md`.

## Layout
- `autoresearch/`: parallel autonomous research loops. If you were launched as a lane, follow
  `autoresearch/program.md` exactly; it overrides general guidance here.
- `infra/cloud/`: cloud environment setup script, SessionStart hook, network allowlist.
- `infra/secrets/`: one-time Doppler/Modal bootstrap (humans only).
- `docs/agents.md`: how agents, secrets and compute fit together.

## Conventions
- Python ≥ 3.11 with uv. `uv sync`, then `uv run …`. No bare `pip install`.
- Lint/format: `uv run --group dev ruff check . && uv run --group dev ruff format .`
- Tests: `uv run --group dev pytest -q`
- Use modern typing (`list[str]`, `X | None`) and keep modules small.

## Working in parallel
- Never commit to `main` directly. Branch (`feat/…`, `fix/…`, `ar/<task>/<lane>` for research
  lanes), push, and open a PR.
- Stay within the scope of your task. Other agents are editing other parts of the tree.
- Secrets arrive as env vars from Doppler (cloud: SessionStart hook). Never print, log, or commit them.
- GPU work goes through Modal (`autoresearch` `modal` backend), because cloud VMs have no GPU.
