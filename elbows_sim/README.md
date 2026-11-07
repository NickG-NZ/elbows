# elbows-sim

Simulator-agnostic robot environments. Tasks, policies and evaluation are written once, and physics
backends are swapped underneath. MuJoCo (CPU) runs today. Isaac Lab and MJWarp (GPU MuJoCo) are
planned. Training code lives in [`elbows_learn/`](../elbows_learn/README.md), which depends on this package and
never the other way round.

```bash
uv run elbows-sim list
uv run --group dev pytest -q elbows_sim/tests
```

```python
import elbows_sim

env = elbows_sim.make("reach", backend="mujoco", num_envs=64)
obs = env.reset(seed=0)
res = env.step(policy.act(obs))  # res.obs, res.reward, res.terminated, res.truncated, res.info
```

## Layers

```
 consumers (elbows_learn, eval)    see only VecEnv + Policy + (later) the dataset format
            │  VecEnv: batched torch obs dicts, actions in [-1, 1], auto-reset, terminated vs truncated
 TaskEnv (core/env.py)             generic glue: episode counters, auto-reset, episode stats
            │
 Task (tasks/*.py)                 reward / obs / reset, written against the Sim interface only
            │  Sim: joint_pos/vel, frame_pos, set_joint_state, set_marker_pos, set_action, step, render
 Sim backends (backends/*.py)      mujoco_cpu.py today; isaac.py, mjwarp.py later
            │
 Scene assets (tasks/assets/)      MJCF today, USD later, with the same joint/site/camera names
```

Rules that keep the swap cheap. `tests/test_layering.py` enforces the first two:

- **Only `backends/` imports a simulator, and nothing here imports `elbows_learn`.**
- **Core dependencies are only torch and numpy,** so `pip install -e elbows_sim` works inside Isaac Sim's Python.
- **Everything is a batched torch tensor on `sim.device`.** The CPU backend returns CPU tensors, and
  Isaac will return CUDA tensors.
- **Positions are env-local.** Backends that tile envs on a grid (Isaac) subtract env origins.
- **Actions are normalised to [-1, 1].** Backends map them onto actuator ranges.
- **Names are the contract between assets and tasks.** `SceneSpec` lists actuated joints, frames,
  markers and cameras by name.
- **Randomness comes from a CPU `torch.Generator` in the task.** The same seed gives the same
  episodes on every backend.

## Adding a task

Subclass `Task` (see `tasks/reach.py`), declare a `SceneSpec`, and register it in `tasks/__init__.py`.
Observation convention: `"policy"` is the actor input. Add `"critic"` for privileged state and image
keys (uint8 `[N, H, W, 3]`) for vision policies. `instruction` holds the language goal for VLAs.

## Adding a backend (Isaac Lab, MJWarp, ...)

1. Implement `Sim` in `backends/<name>.py`. `backends/isaac.py` has the method-by-method mapping to
   Isaac Lab APIs.
2. Register it in `backends/__init__.py` and make `available()` return true when it is importable.
3. Provide the scene in a format the backend can load (add e.g. `"usd": ...` to `SceneSpec.assets`).
4. Run `pytest elbows_sim/tests/test_backend_contract.py`. Every test is parametrised over available
   backends, including an analytic forward-kinematics check, so a passing backend runs every task
   and trained policy unchanged.

Isaac runs in its own Python on Linux with an RTX GPU (GCP G2/L4 or G4; not A100/H100).

## Policy interface

Anything with `act(obs: dict[str, Tensor]) -> Tensor` and `reset(env_ids)` is a `Policy`
(`policies.py`), whether an RL MLP, a VLA with action chunking, or a world-model planner.
`evaluate()` works with all of them.
