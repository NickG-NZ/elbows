"""The environment API that learners, policies and evaluation code use.

`VecEnv` is deliberately close to Isaac Lab's vectorised env semantics: batched torch tensors,
done envs are reset automatically inside `step()`, and `terminated` / `truncated` are separate so
learners can bootstrap through time limits.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field

import torch

from .sim import Sim
from .spec import EnvSpec
from .task import Task

Obs = dict[str, torch.Tensor]


@dataclass
class StepResult:
    obs: Obs  # for envs that just finished, this is the first obs of their next episode
    reward: torch.Tensor  # [N]
    terminated: torch.Tensor  # [N] bool, episode ended by the task (failure/success)
    truncated: torch.Tensor  # [N] bool, episode ended by the time limit
    info: dict = field(default_factory=dict)  # info["episode"]: stats of envs that finished this step


class VecEnv(ABC):
    num_envs: int
    device: torch.device
    spec: EnvSpec

    @abstractmethod
    def reset(self, seed: int | None = None) -> Obs: ...

    @abstractmethod
    def step(self, action: torch.Tensor) -> StepResult: ...

    @abstractmethod
    def render(self, camera: str | None = None, env_ids: torch.Tensor | None = None, **kw) -> torch.Tensor: ...

    def close(self) -> None:
        pass


class TaskEnv(VecEnv):
    """Generic glue between a Task and any Sim backend."""

    def __init__(self, task: Task, sim: Sim, seed: int = 0):
        self.task, self.sim = task, sim
        self.num_envs, self.device = sim.num_envs, sim.device
        task.bind(self.num_envs, self.device)
        self.rng = torch.Generator().manual_seed(seed)
        self.episode_len = torch.zeros(self.num_envs, dtype=torch.long, device=self.device)
        self.episode_return = torch.zeros(self.num_envs, device=self.device)
        self.spec = EnvSpec(
            observations=task.observation_spec(),
            action_dim=len(task.scene.actuated_joints),
            control_dt=task.scene.control_dt,
            max_episode_steps=task.max_episode_steps,
            instruction=task.instruction,
            metadata={"task": type(task).__name__, "backend": type(sim).__name__},
        )

    def reset(self, seed: int | None = None) -> Obs:
        if seed is not None:
            self.rng.manual_seed(seed)
        self._reset_envs(torch.arange(self.num_envs, device=self.device))
        return self.task.observe(self.sim)

    def step(self, action: torch.Tensor) -> StepResult:
        action = action.to(self.device, torch.float32).clamp(-1.0, 1.0)
        self.sim.set_action(action)
        self.sim.step()
        reward = self.task.reward(self.sim, action)
        terminated = self.task.terminated(self.sim)
        self.episode_len += 1
        self.episode_return += reward
        truncated = (self.episode_len >= self.task.max_episode_steps) & ~terminated
        done = terminated | truncated

        info: dict = {}
        if done.any():
            ids = done.nonzero().squeeze(-1)
            episode = {"return": self.episode_return[ids].clone(), "length": self.episode_len[ids].clone()}
            episode |= {k: v[ids].clone() for k, v in self.task.metrics(self.sim).items()}
            info["episode"] = episode
            self._reset_envs(ids)
        return StepResult(self.task.observe(self.sim), reward, terminated, truncated, info)

    def render(self, camera: str | None = None, env_ids: torch.Tensor | None = None, **kw) -> torch.Tensor:
        camera = camera or self.task.scene.cameras[0]
        return self.sim.render(camera, env_ids, **kw)

    def close(self) -> None:
        self.sim.close()

    def _reset_envs(self, ids: torch.Tensor) -> None:
        self.task.reset(self.sim, ids, self.rng)
        self.episode_len[ids] = 0
        self.episode_return[ids] = 0.0
