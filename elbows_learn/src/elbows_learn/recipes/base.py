"""A recipe says how to learn one task: which task (from elbows_sim), which method, and its settings.

Tasks define *what* to do and know nothing about training; recipes hold everything task-specific
about *how* to learn it. One task can have many recipes (state vs. vision, PPO vs. imitation, quick
smoke test vs. full run).
"""

from __future__ import annotations

from dataclasses import asdict, dataclass, field

from ..rl import PPOConfig


@dataclass(frozen=True)
class Recipe:
    name: str
    task: str  # a key of elbows_sim.TASKS
    description: str
    ppo: PPOConfig  # the training method; becomes a union of configs once VLA / world-model training lands
    num_envs: int = 64
    backend: str = "mujoco"  # default only; any backend that implements the task works
    task_kwargs: dict = field(default_factory=dict)
    eval_episodes: int = 32

    def to_dict(self) -> dict:
        return asdict(self)
