# elbows

Robotics monorepo, set up for many coding and research agents working in parallel.

| path | what |
|---|---|
| [`autoresearch/`](autoresearch/README.md) | parallel autonomous research loops (after karpathy/autoresearch) |
| [`docs/agent-infra.md`](docs/agent-infra.md) | setup guide: Claude Code cloud + Doppler + Modal |
| `infra/` | cloud environment scripts and secrets bootstrap |

```bash
uv sync
uv run ar run cartpole-swingup -m "baseline"
```
