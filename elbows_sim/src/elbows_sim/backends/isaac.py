"""Isaac Lab backend: placeholder.

Implement this when moving to Isaac. It runs in Isaac Sim's own Python (Linux + NVIDIA RTX GPU, e.g. a
GCP G2/L4 or G4/RTX PRO 6000 VM; A100/H100 have no RT cores and can't render). Install elbows-sim into
that environment with `pip install -e elbows_sim` (core deps are only torch + numpy).

Mapping from the `Sim` interface to Isaac Lab. API names are from Isaac Lab 2.x, so check them
against the installed version (3.x adds the Newton backend, whose MuJoCo-Warp solver should keep
physics close to the MuJoCo backend):

- __init__        Start `isaaclab.app.AppLauncher(headless=True, enable_cameras=...)` BEFORE importing
                  any other isaaclab module. Build an `InteractiveScene` with `num_envs` clones of the
                  scene's "usd" asset (or convert the "mjcf" one with Isaac's MJCF importer), keeping
                  joint/site/camera names identical. Set sim dt = scene.physics_dt.
- set_action      Map [-1, 1] to effort limits -> `articulation.set_joint_effort_target(...)`. Match the
                  MJCF actuator type: motor -> effort, position -> `set_joint_position_target`.
- step            Repeat `scene.decimation` times: `scene.write_data_to_sim(); sim.step()`, then
                  `scene.update(dt)`.
- joint_pos/vel   `articulation.data.joint_pos[:, joint_ids]` / `joint_vel[:, joint_ids]`.
- frame_pos       Body or frame world position minus `scene.env_origins`. Positions must be env-local.
                  MJCF sites have no USD equivalent; add an Xform prim or a FrameTransformer.
- joint_limits    `articulation.data.joint_pos_limits[0, joint_ids]`.
- reset           `articulation.write_root_state_to_sim(default_root_state + env_origins)` and
                  `write_joint_state_to_sim(default_joint_pos, default_joint_vel, env_ids=...)`.
- set_joint_state `articulation.write_joint_state_to_sim(pos, vel, joint_ids=..., env_ids=...)`.
- set_marker_pos  A kinematic rigid object (or VisualizationMarkers for visual-only): write pose + env origin.
- render          `TiledCamera` with the same intrinsics as the MJCF camera; `.data.output["rgb"]`.

Then run `pytest elbows_sim/tests/test_backend_contract.py`. The contract tests are parametrised over
every available backend, including checks that the same task gives the same kinematics.
"""

from __future__ import annotations

import torch

from ..core.sim import Sim
from ..core.spec import SceneSpec


class IsaacLabSim(Sim):
    REQUIRES = "isaaclab"

    @classmethod
    def available(cls) -> bool:
        return False  # flip to `super().available()` once implemented

    def __init__(self, scene: SceneSpec, num_envs: int, device: str | torch.device | None = None):
        raise NotImplementedError(
            "Isaac Lab backend not implemented yet; see elbows_sim/src/elbows_sim/backends/isaac.py"
        )

    def set_action(self, action):
        raise NotImplementedError  # noqa: E704

    def step(self):
        raise NotImplementedError  # noqa: E704

    def joint_pos(self):
        raise NotImplementedError  # noqa: E704

    def joint_vel(self):
        raise NotImplementedError  # noqa: E704

    def frame_pos(self, name):
        raise NotImplementedError  # noqa: E704

    def joint_limits(self):
        raise NotImplementedError  # noqa: E704

    def reset(self, env_ids):
        raise NotImplementedError  # noqa: E704

    def set_joint_state(self, env_ids, pos, vel):
        raise NotImplementedError  # noqa: E704

    def set_marker_pos(self, name, env_ids, pos):
        raise NotImplementedError  # noqa: E704

    def render(self, camera, env_ids=None, width=256, height=256):
        raise NotImplementedError  # noqa: E704
