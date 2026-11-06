"""Batched cart-pole swing-up simulator (torch, CPU). FROZEN: agents must not edit this file.

theta = 0 is upright, theta = pi is hanging down. The episode starts near hanging; the policy has to
pump energy in, swing the pole up, and balance it while keeping the cart on the track.
Physical parameters are per-environment so evaluation can randomise them (a sim-to-real proxy).
"""

from __future__ import annotations

import math

import torch

G = 9.81
DT = 0.02
HORIZON = 500  # 10 s
FORCE_MAX = 10.0
X_LIMIT = 2.4
OBS_DIM = 5  # x, x_dot, cos(theta), sin(theta), theta_dot
ACT_DIM = 1  # force in [-1, 1] * FORCE_MAX
NOMINAL = {"cart_mass": 1.0, "pole_mass": 0.1, "half_length": 0.5, "friction": 0.0}


class CartPole:
    def __init__(self, n: int, generator: torch.Generator, param_noise: float = 0.0, device: str = "cpu"):
        self.n = n
        self.g = generator
        self.device = device

        def jitter(v: float) -> torch.Tensor:
            u = torch.rand(n, generator=generator, device=device) * 2 - 1
            return v * (1 + param_noise * u)

        self.mc = jitter(NOMINAL["cart_mass"])
        self.mp = jitter(NOMINAL["pole_mass"])
        self.l = jitter(NOMINAL["half_length"])
        self.friction = torch.rand(n, generator=generator, device=device) * param_noise * 0.5
        self.state = torch.zeros(n, 4, device=device)
        self.alive = torch.ones(n, dtype=torch.bool, device=device)

    def reset(self) -> torch.Tensor:
        u = lambda s: (torch.rand(self.n, generator=self.g, device=self.device) * 2 - 1) * s  # noqa: E731
        self.state = torch.stack([u(0.5), u(0.1), math.pi + u(0.3), u(0.1)], dim=-1)
        self.alive = torch.ones(self.n, dtype=torch.bool, device=self.device)
        return self.obs()

    def obs(self) -> torch.Tensor:
        x, xd, th, thd = self.state.unbind(-1)
        return torch.stack([x, xd, torch.cos(th), torch.sin(th), thd], dim=-1)

    def step(self, action: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
        """action: [n] or [n, 1] in [-1, 1]. Returns obs, reward [n], alive mask [n]."""
        a = action.reshape(self.n).clamp(-1, 1)
        a = torch.nan_to_num(a, nan=0.0)
        f = a * FORCE_MAX
        x, xd, th, thd = self.state.unbind(-1)
        s, c = torch.sin(th), torch.cos(th)
        mt = self.mc + self.mp
        tmp = (f - self.friction * xd + self.mp * self.l * thd**2 * s) / mt
        thacc = (G * s - c * tmp) / (self.l * (4.0 / 3.0 - self.mp * c**2 / mt))
        xacc = tmp - self.mp * self.l * thacc * c / mt
        xd = xd + DT * xacc
        x = x + DT * xd
        thd = thd + DT * thacc
        th = th + DT * thd
        self.state = torch.stack([x, xd, th, thd], dim=-1)

        upright = (1 + torch.cos(th)) / 2  # 1 when upright, 0 when hanging
        centred = 1 - (x / X_LIMIT).clamp(-1, 1) ** 2
        reward = upright * (0.8 + 0.2 * centred) - 0.01 * a**2
        reward = torch.where(self.alive, reward, torch.zeros_like(reward))
        self.alive = self.alive & (x.abs() <= X_LIMIT)
        return self.obs(), reward, self.alive


def rollout(policy, env: CartPole, horizon: int = HORIZON) -> torch.Tensor:
    """Total return per environment for a policy mapping obs [n, 5] -> action [n] or [n, 1]."""
    obs = env.reset()
    total = torch.zeros(env.n, device=env.device)
    for _ in range(horizon):
        obs, r, _ = env.step(policy(obs))
        total += r
    return total
