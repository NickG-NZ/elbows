# elbows

Robotics monorepo. Many coding agents (Claude Code, Codex, …) work here in parallel, locally and in the cloud.
This file is the shared agent guide; `CLAUDE.md` is a symlink to it, so edit `AGENTS.md`.

## Layout
- `elbows_sim/`: environments. Tasks (what to do) and simulator backends behind one `Sim` interface.
  Only `backends/` may import a simulator; new backends must pass `tests/test_backend_contract.py`.
- `elbows_learn/`: training. Generic methods plus `recipes/` (how to learn a given task). Depends on
  `elbows_sim`, never the reverse (`elbows_sim/tests/test_layering.py`).
- `autoresearch/`: parallel autonomous research loops. If you were launched as a lane, follow
  `autoresearch/program.md` exactly; it overrides general guidance here.
- `infra/cloud/`: cloud environment setup script, SessionStart hook, network allowlist.
- `infra/secrets/`: one-time Doppler/Modal bootstrap (humans only).
- `docs/agent-infra.md`: setup guide for agent infrastructure (cloud sessions, secrets, compute).

## Conventions
- Python ≥ 3.11 with uv. `uv sync`, then `uv run …`. No bare `pip install`.
- Lint/format: `uv run --group dev ruff check . && uv run --group dev ruff format .`
- Tests: `uv run --group dev pytest -q`
- Use modern typing (`list[str]`, `X | None`) and keep modules small.

## Working in parallel
- Git depends on how you were launched:
  - **Autonomous parallel agents** (autoresearch lanes, batch-launched cloud sessions): never
    commit to `main`. Branch (`ar/<task>/<lane>`, `feat/…`, `fix/…`), push, and open a PR.
  - **Interactive sessions working directly with the user:** commit and push to `main`. Pull or
    rebase first, and never force-push.
- Stay within the scope of your task. Other agents are editing other parts of the tree.
- GPU work goes through Modal (`autoresearch` `modal` backend), because cloud VMs have no GPU.

## Secrets
- Doppler, project `elbows`. No `.env` files. Never print, log, or commit secrets.
- Locally: `doppler run -- <cmd>`. Cloud sessions get them via the SessionStart hook.
