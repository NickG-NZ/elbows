# elbows

Robotics monorepo for end-to-end control policies and neural simulators, set up for many coding and
research agents working in parallel.

| path | what |
|---|---|
| [`elbows_sim/`](elbows_sim/README.md) | simulator-agnostic environments and tasks (MuJoCo now, Isaac Lab later) |
| [`elbows_learn/`](elbows_learn/README.md) | training methods and per-task recipes, built on `elbows_sim` |
| [`autoresearch/`](autoresearch/README.md) | parallel autonomous research loops (after karpathy/autoresearch) |
| [`docs/agent-infra.md`](docs/agent-infra.md) | setup guide: Claude Code cloud + Doppler + Modal |
| `infra/` | cloud environment scripts and secrets bootstrap |

```bash
uv sync
uv run elbows-learn train reach_ppo       # PPO on a 2-link arm, ~1 min on a laptop CPU
uv run ar run cartpole-swingup -m "baseline"
```
