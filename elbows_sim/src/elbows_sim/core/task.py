"""Tasks: reward, observation and reset logic written once against the `Sim` interface."""

from __future__ import annotations

from abc import ABC, abstractmethod

import torch

from .sim import Sim
from .spec import SceneSpec, TensorSpec


class Task(ABC):
    scene: SceneSpec
    max_episode_steps: int = 200
    instruction: str = ""

    def bind(self, num_envs: int, device: torch.device) -> None:
        """Allocate per-env task buffers (targets, goals, ...). Called once by TaskEnv."""
        self.num_envs = num_envs
        self.device = device

    @abstractmethod
    def observation_spec(self) -> dict[str, TensorSpec]:
        """By convention "policy" is the actor's input; add "critic" for privileged values, images, etc."""

    @abstractmethod
    def reset(self, sim: Sim, env_ids: torch.Tensor, rng: torch.Generator) -> None:
        """Sample initial states and task parameters for `env_ids` and write them into the sim.

        Sample with `rng` (a CPU generator) and move to `sim.device`, so episodes are reproducible
        across backends and devices.
        """

    @abstractmethod
    def observe(self, sim: Sim) -> dict[str, torch.Tensor]: ...

    @abstractmethod
    def reward(self, sim: Sim, action: torch.Tensor) -> torch.Tensor: ...

    def terminated(self, sim: Sim) -> torch.Tensor:
        return torch.zeros(sim.num_envs, dtype=torch.bool, device=sim.device)

    def metrics(self, sim: Sim) -> dict[str, torch.Tensor]:
        """Per-env scalars reported at the end of each episode (e.g. success)."""
        return {}
