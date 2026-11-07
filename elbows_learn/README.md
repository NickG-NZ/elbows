# elbows-learn

Model training on top of [`elbows-sim`](../elbows_sim/README.md). PPO today; VLAs, world models and
neural simulators later.

```bash
uv run elbows-learn list                      # recipes
uv run elbows-learn train reach_ppo           # ~1 min on a laptop CPU -> runs/reach_ppo-mujoco-s0/
# once the Isaac backend exists: same recipe, other simulator
# uv run elbows-learn train reach_ppo --backend isaac --num-envs 4096
uv run elbows-learn eval runs/reach_ppo-mujoco-s0/policy.pt --video runs/reach.mp4
uv run --group dev pytest -q elbows_learn/tests
```

## What goes where

| | lives in | example |
|---|---|---|
| **what** to do: task, reward, observations, success | `elbows_sim/tasks/` | `reach.py` |
| **how** to learn it: method + task-specific settings | `elbows_learn/recipes/` | `reach_ppo.py` |
| generic methods and models, task-agnostic | `elbows_learn/rl/` (later `vla/`, `world_models/`) | `ppo.py` |

- A **recipe** names a task, a method and its settings (budget, network size, env count, task
  arguments). One task can have several, e.g. `reach_ppo` and `reach_ppo_smoke`.
- **Methods** only see `elbows_sim`'s `VecEnv` / `Policy` interfaces (and, for offline training,
  datasets), so they work on any task and any backend.
- Command-line flags override a recipe for one run. Each run writes the resolved recipe to
  `runs/<name>/recipe.json`, and the checkpoint stores it too, so `eval` rebuilds the same task.

To add a recipe, add a `Recipe` to a module in `recipes/` and register it in `recipes/__init__.py`.
`tests/test_recipes.py` checks that every recipe points at a real task and runs one iteration.

Core dependencies stay light (torch plus elbows-sim) so online RL can run inside Isaac Sim's Python.
Put heavy dependencies for offline model training (transformers, lerobot, ...) in optional extras.
