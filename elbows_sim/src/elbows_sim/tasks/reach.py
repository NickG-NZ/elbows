"""Reach: move a planar 2-link arm's fingertip to a random target."""

from __future__ import annotations

import math
from pathlib import Path

import torch

from ..core.sim import Sim
from ..core.spec import SceneSpec, TensorSpec
from ..core.task import Task

ASSETS = Path(__file__).parent / "assets"
LINK_LENGTHS = (0.2, 0.2)


class Reach(Task):
    scene = SceneSpec(
        name="arm2",
        assets={"mjcf": ASSETS / "arm2.xml"},
        actuated_joints=("shoulder", "elbow"),
        frames=("tip",),
        markers=("target",),
        cameras=("top",),
        physics_dt=0.005,
        decimation=4,  # 50 Hz control
    )
    instruction = "move the fingertip to the red ball"

    def __init__(
        self,
        max_episode_steps: int = 150,
        target_radius: tuple[float, float] = (0.08, 0.38),
        success_threshold: float = 0.02,
    ):
        self.max_episode_steps = max_episode_steps
        self.target_radius = target_radius
        self.success_threshold = success_threshold

    def bind(self, num_envs: int, device: torch.device) -> None:
        super().bind(num_envs, device)
        self.target = torch.zeros(num_envs, 3, device=device)

    def observation_spec(self) -> dict[str, TensorSpec]:
        return {"policy": TensorSpec((12,))}

    def reset(self, sim: Sim, env_ids: torch.Tensor, rng: torch.Generator) -> None:
        n = len(env_ids)
        u = lambda *shape: torch.rand(*shape, generator=rng)
        sim.reset(env_ids)
        q = torch.stack([(u(n) * 2 - 1) * math.pi, (u(n) * 2 - 1) * 2.0], dim=-1)
        sim.set_joint_state(env_ids, q.to(sim.device), torch.zeros(n, 2, device=sim.device))

        lo, hi = self.target_radius
        r = torch.sqrt(lo**2 + (hi**2 - lo**2) * u(n))  # uniform over the annulus' area
        theta = u(n) * 2 * math.pi
        target = torch.stack([r * torch.cos(theta), r * torch.sin(theta), torch.zeros(n)], dim=-1).to(sim.device)
        self.target[env_ids] = target
        sim.set_marker_pos("target", env_ids, target)

    def _distance(self, sim: Sim) -> torch.Tensor:
        return torch.linalg.norm(sim.frame_pos("tip")[:, :2] - self.target[:, :2], dim=-1)

    def observe(self, sim: Sim) -> dict[str, torch.Tensor]:
        q, qd = sim.joint_pos(), sim.joint_vel()
        tip, target = sim.frame_pos("tip")[:, :2], self.target[:, :2]
        return {"policy": torch.cat([q.cos(), q.sin(), 0.1 * qd, tip, target, target - tip], dim=-1)}

    def reward(self, sim: Sim, action: torch.Tensor) -> torch.Tensor:
        d = self._distance(sim)
        return 1 - torch.tanh(5 * d) - 0.01 * (action**2).sum(-1) - 0.001 * (sim.joint_vel() ** 2).sum(-1)

    def metrics(self, sim: Sim) -> dict[str, torch.Tensor]:
        d = self._distance(sim)
        return {"final_distance": d, "success": (d < self.success_threshold).float()}
