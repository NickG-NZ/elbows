# elbows-sim

Simulator-agnostic robot environments. Tasks, policies and evaluation are written once, and physics
backends are swapped underneath. MuJoCo (CPU) and MJX (MuJoCo in JAX, CPU or GPU) run today. Isaac Lab and
MJWarp are planned. Training code lives in [`elbows_learn/`](../elbows_learn/README.md), which depends on this package and
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
            │  Sim: joint_pos/vel, frame_pos/rot, set_joint_state, set_marker_pos, set_action, step, render
 Sim backends (backends/*.py)      mujoco_cpu.py, mjx.py today; isaac.py, mjwarp.py later
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

## MJX backend

`uv sync --extra mjx`, then `backend="mjx"`. It installs `jax[cuda12]` on Linux and uses the GPU when
JAX finds one. On CPU it is slower than `mujoco`, so use it for thousands of envs on a GPU. Reach
on an L4 runs at about 200k env-steps/s with 4,096 envs and 760k with 16,384, versus about 22k for
`mujoco` with 64 envs. JIT compilation takes a few seconds per scene and env count.

## Robots from URDF

A task can load its robot from a URDF instead of a hand-written MJCF (`SceneSpec.robot`). The SO-101
uses this. Pass any URDF of a modified design, e.g. one with a forearm roll:

```python
env = elbows_sim.make("so101_pose", urdf="~/Documents/so101-6dof/urdf/so101_6dof.urdf")
env.task.scene.actuated_joints  # read from the URDF in kinematic-tree order
```

- **Joints and limits come from the URDF.** Actuated joints are ordered by walking the tree from the
  root, so an added joint lands where it sits in the chain and the action vector grows by one.
  Every actuated joint needs finite limits.
- **Actuators are a TOML file in the robot's folder** (`robots/so101/actuators.toml`): a `[default]`
  for every joint, plus `[joints.<name>]` for any joint that differs. Each table's `type` picks a
  kind from `core/actuators.py` (`position_servo`, `dc_motor`); a modified arm can pass its own
  file: `so101_robot(urdf, actuators="~/so101-6dof/actuators.toml")`. Each backend translates every
  kind (`backends/mujoco_actuators.py`), so the same file drives every simulator. New kinds
  (hydraulic, series-elastic, ...) are a dataclass plus a case per backend.
- **SO-101 servo model (STS3215):** a position loop whose torque is clipped at the datasheet stall
  torque, plus back-EMF damping (`stall_torque / no_load_speed`). Together they give the DC motor's
  linear torque-speed curve, so joints can't exceed the datasheet no-load speed.
- **Two limits per joint.** The URDF limits are the *command* range: an action in [-1, 1] is a
  target position across it, so policies can't command past it. `hard_stops.toml` in the
  robot's folder holds the *physical* end stops, where the joint actually stops in sim. The
  SO-101's wrist roll stops (-163.4, +170.4 deg) come from a CAD sweep: `scripts/hard_stop_sweep.py --joint ...`
  finds them for any URDF joint from its meshes.
- **Contacts:** self-collision and floor contact are on. Adjacent links are excluded, since their
  meshes overlap at the joint. Collision uses each mesh's convex hull capped at 64 vertices
  (`HULL_VERTS`), which is slightly conservative: about 5% of self-contacts are near misses inside
  a hull. `sim.robot_contacts()` returns per-env [self, scene] contact counts. `so101_dance` ends
  an episode on any contact. On MJX, mesh self-collision makes the first compile slow (minutes).
- **Meshes:** relative paths resolve from the URDF's folder, and `package://<pkg>/x.stl` resolves to
  `<urdf folder>/x.stl`. MuJoCo reads binary STL and OBJ.
- **Names are the contract.** Policies depend on joint names and order, so keep the stock names
  when modifying the arm and only add new ones.
- **Layout:** one folder per robot, `robots/<robot>/`: its builder, `actuators.toml`, `hard_stops.toml`
  and the unmodified upstream URDF and meshes (`robots/so101/README.md`). Shared surroundings are in
  `scenes/`.

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
