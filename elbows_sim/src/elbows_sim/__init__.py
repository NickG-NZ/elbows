"""elbows_sim: simulator-agnostic robot environments. `make()` is the main entry point."""

from __future__ import annotations

import torch

from .backends import available_backends, make_sim
from .core import EnvSpec, SceneSpec, Sim, StepResult, Task, TaskEnv, TensorSpec, VecEnv
from .tasks import TASKS


def make(
    task: str,
    backend: str = "mujoco",
    num_envs: int = 1,
    device: str | torch.device | None = None,
    seed: int = 0,
    **task_kwargs,
) -> TaskEnv:
    """Build a vectorised env, e.g. `elbows_sim.make("reach", backend="mujoco", num_envs=64)`."""
    if task not in TASKS:
        raise KeyError(f"unknown task {task!r}; choose from {sorted(TASKS)}")
    t = TASKS[task](**task_kwargs)
    return TaskEnv(t, make_sim(backend, t.scene, num_envs, device), seed=seed)


__all__ = [
    "TASKS",
    "EnvSpec",
    "SceneSpec",
    "Sim",
    "StepResult",
    "Task",
    "TaskEnv",
    "TensorSpec",
    "VecEnv",
    "available_backends",
    "make",
    "make_sim",
]
