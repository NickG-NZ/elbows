"""Backend-neutral descriptions of scenes, observations and environments."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

import torch


@dataclass(frozen=True)
class TensorSpec:
    """Shape and dtype of one observation entry, per environment (no batch dim)."""

    shape: tuple[int, ...]
    dtype: torch.dtype = torch.float32


@dataclass(frozen=True)
class SceneSpec:
    """Everything a backend needs to build a scene, without saying how.

    `assets` maps an asset format to a file, e.g. {"mjcf": ..., "usd": ...}. Each backend picks the
    format it can load. When adding Isaac, either author a USD next to the MJCF or convert the MJCF
    with Isaac's importer, and keep joint/site/camera names identical so tasks work unchanged.
    """

    name: str
    assets: dict[str, Path]
    actuated_joints: tuple[str, ...]  # order defines the action vector
    frames: tuple[str, ...] = ()  # named points tasks read world positions of (MJCF sites/bodies)
    markers: tuple[str, ...] = ()  # kinematic, non-colliding bodies tasks can move (targets, goals)
    cameras: tuple[str, ...] = ()
    physics_dt: float = 0.005
    decimation: int = 4  # physics steps per control step

    @property
    def control_dt(self) -> float:
        return self.physics_dt * self.decimation


@dataclass(frozen=True)
class EnvSpec:
    """What a policy or learner may rely on. Actions are always normalised to [-1, 1]."""

    observations: dict[str, TensorSpec]
    action_dim: int
    control_dt: float
    max_episode_steps: int
    instruction: str = ""  # natural-language task description, for language-conditioned policies
    metadata: dict[str, object] = field(default_factory=dict)
