"""The backend interface. This is the only thing a new simulator (e.g. Isaac Lab) has to implement.

Conventions every backend must follow (checked by tests/test_backend_contract.py):
- Everything is batched over `num_envs` and returned as torch tensors on `self.device`.
- Positions are env-local: each env's origin is (0, 0, 0) even if the backend lays envs out on a grid.
- Actions are normalised to [-1, 1] and mapped linearly onto each actuator's control range.
- `env_ids` is a 1-D long tensor of env indices.
- Reads after `step()` or any setter reflect the current state (backends sync kinematics as needed).
"""

from __future__ import annotations

import importlib.util
from abc import ABC, abstractmethod

import torch

from .spec import SceneSpec


class Sim(ABC):
    #: Python module the backend needs; used by `available()`.
    REQUIRES: str | None = None

    def __init__(self, scene: SceneSpec, num_envs: int, device: torch.device):
        self.scene = scene
        self.num_envs = num_envs
        self.device = device

    @classmethod
    def available(cls) -> bool:
        return cls.REQUIRES is None or importlib.util.find_spec(cls.REQUIRES) is not None

    @property
    def control_dt(self) -> float:
        return self.scene.control_dt

    # --- control ---------------------------------------------------------------------------------
    @abstractmethod
    def set_action(self, action: torch.Tensor) -> None:
        """Apply normalised actions [num_envs, num_actuated_joints] in [-1, 1]."""

    @abstractmethod
    def step(self) -> None:
        """Advance all envs by one control step (`scene.decimation` physics steps)."""

    # --- state reads -----------------------------------------------------------------------------
    @abstractmethod
    def joint_pos(self) -> torch.Tensor:
        """[num_envs, num_actuated_joints] joint positions."""

    @abstractmethod
    def joint_vel(self) -> torch.Tensor:
        """[num_envs, num_actuated_joints] joint velocities."""

    @abstractmethod
    def frame_pos(self, name: str) -> torch.Tensor:
        """[num_envs, 3] env-local position of a named frame from `scene.frames`."""

    @abstractmethod
    def joint_limits(self) -> torch.Tensor:
        """[num_actuated_joints, 2] lower/upper limits (+-inf when unlimited)."""

    # --- state writes ----------------------------------------------------------------------------
    @abstractmethod
    def reset(self, env_ids: torch.Tensor) -> None:
        """Restore the scene's default state (all bodies, controls) for these envs."""

    @abstractmethod
    def set_joint_state(self, env_ids: torch.Tensor, pos: torch.Tensor, vel: torch.Tensor) -> None:
        """Set actuated joint positions/velocities, each [len(env_ids), num_actuated_joints]."""

    @abstractmethod
    def set_marker_pos(self, name: str, env_ids: torch.Tensor, pos: torch.Tensor) -> None:
        """Move a kinematic marker from `scene.markers` to env-local positions [len(env_ids), 3]."""

    # --- sensors ---------------------------------------------------------------------------------
    @abstractmethod
    def render(
        self, camera: str, env_ids: torch.Tensor | None = None, width: int = 256, height: int = 256
    ) -> torch.Tensor:
        """uint8 RGB images [len(env_ids), height, width, 3] from a camera in `scene.cameras`."""

    def close(self) -> None:
        pass
