"""Policies map batched observation dicts to normalised actions in [-1, 1].

Every policy, whether an MLP from RL, a VLA, or a world-model planner, implements this one interface,
so evaluation and data collection code works for all of them on any backend.
"""

from __future__ import annotations

from typing import Protocol, runtime_checkable

import torch


@runtime_checkable
class Policy(Protocol):
    def act(self, obs: dict[str, torch.Tensor]) -> torch.Tensor:
        """[num_envs, action_dim] actions for a batch of observations."""
        ...

    def reset(self, env_ids: torch.Tensor | None = None) -> None:
        """Clear per-env internal state (action chunks, history). No-op for stateless policies."""
        ...
