"""SO-101 pose: drive every joint to a random target pose. The building block for pose-sequence
tasks such as dancing.

Works with the stock arm or any modified URDF: `make("so101_pose", urdf="~/so101_6dof.urdf")`.
Actions and observations size themselves to the URDF's actuated joints (tree order).
"""

from __future__ import annotations

from pathlib import Path

import torch

from ..core.sim import Sim
from ..core.spec import SceneSpec, TensorSpec
from ..core.task import Task
from ..formats.urdf import urdf_joints
from ..robots.so101 import SCENE, so101_robot


class So101Pose(Task):
    instruction = "move the arm to the target pose"

    def __init__(
        self,
        urdf: str | Path | None = None,
        max_episode_steps: int = 100,
        range_fraction: float = 0.8,
        success_threshold: float = 0.05,
        self_collision: bool = False,
    ):
        robot = so101_robot(urdf, self_collision=self_collision)
        joints = urdf_joints(robot.urdf)
        self.scene = SceneSpec(
            name=f"so101:{robot.urdf.stem}",
            assets={"mjcf": SCENE},
            actuated_joints=tuple(j.name for j in joints),
            cameras=("front", "top"),
            physics_dt=0.005,
            decimation=4,  # 50 Hz control
            robot=robot,
        )
        lo = torch.tensor([j.lower for j in joints])
        hi = torch.tensor([j.upper for j in joints])
        self._mid, self._half = (hi + lo) / 2, (hi - lo) / 2
        self.max_episode_steps = max_episode_steps
        self.range_fraction = range_fraction
        self.success_threshold = success_threshold

    @property
    def num_joints(self) -> int:
        return len(self.scene.actuated_joints)

    def bind(self, num_envs: int, device: torch.device) -> None:
        super().bind(num_envs, device)
        self._mid, self._half = self._mid.to(device), self._half.to(device)
        self.target = torch.zeros(num_envs, self.num_joints, device=device)

    def observation_spec(self) -> dict[str, TensorSpec]:
        return {"policy": TensorSpec((3 * self.num_joints,))}

    def _sample_pose(self, n: int, rng: torch.Generator) -> torch.Tensor:
        u = torch.rand(n, self.num_joints, generator=rng) * 2 - 1
        return (self._mid.cpu() + self.range_fraction * self._half.cpu() * u).to(self.device)

    def reset(self, sim: Sim, env_ids: torch.Tensor, rng: torch.Generator) -> None:
        n = len(env_ids)
        sim.reset(env_ids)
        q = self._sample_pose(n, rng)
        sim.set_joint_state(env_ids, q, torch.zeros_like(q))
        self.target[env_ids] = self._sample_pose(n, rng)

    def _normalise(self, q: torch.Tensor) -> torch.Tensor:
        return (q - self._mid) / self._half

    def observe(self, sim: Sim) -> dict[str, torch.Tensor]:
        q = self._normalise(sim.joint_pos())
        target = self._normalise(self.target)
        return {"policy": torch.cat([q, 0.1 * sim.joint_vel(), target - q], dim=-1)}

    def _error(self, sim: Sim) -> torch.Tensor:
        return (sim.joint_pos() - self.target).abs().max(dim=-1).values

    def reward(self, sim: Sim, action: torch.Tensor) -> torch.Tensor:
        err = torch.linalg.norm(sim.joint_pos() - self.target, dim=-1)
        return 1 - torch.tanh(2 * err) - 0.01 * (sim.joint_vel() ** 2).sum(-1)

    def metrics(self, sim: Sim) -> dict[str, torch.Tensor]:
        err = self._error(sim)
        return {"max_joint_error": err, "success": (err < self.success_threshold).float()}
