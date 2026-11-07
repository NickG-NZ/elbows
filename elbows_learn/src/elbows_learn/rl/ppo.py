"""Compact PPO for any `VecEnv` (continuous actions, Gaussian policy, GAE, observation normalisation).

Truncated episodes are bootstrapped with the value of the pre-step state, the same approximation
Isaac Lab's rsl_rl uses. It works with auto-resetting envs that don't expose the final observation.
"""

from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import torch
from torch import nn

from elbows_sim import VecEnv


@dataclass
class PPOConfig:
    total_steps: int = 1_000_000
    rollout_steps: int = 32  # per env, per iteration
    epochs: int = 5
    minibatches: int = 4
    gamma: float = 0.99
    gae_lambda: float = 0.95
    clip: float = 0.2
    lr: float = 3e-4
    ent_coef: float = 0.0
    vf_coef: float = 0.5
    max_grad_norm: float = 1.0
    hidden: tuple[int, ...] = (128, 128)
    init_log_std: float = -0.5
    obs_key: str = "policy"


class RunningNorm(nn.Module):
    def __init__(self, dim: int, eps: float = 1e-8):
        super().__init__()
        self.eps = eps
        self.register_buffer("mean", torch.zeros(dim))
        self.register_buffer("var", torch.ones(dim))
        self.register_buffer("count", torch.tensor(eps))

    @torch.no_grad()
    def update(self, x: torch.Tensor) -> None:
        b_mean, b_var, b_n = x.mean(0), x.var(0, unbiased=False), x.shape[0]
        delta, total = b_mean - self.mean, self.count + b_n
        self.mean += delta * b_n / total
        self.var = (self.var * self.count + b_var * b_n + delta**2 * self.count * b_n / total) / total
        self.count = total

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return ((x - self.mean) / torch.sqrt(self.var + self.eps)).clamp(-10, 10)


def mlp(i: int, hidden: tuple[int, ...], o: int, out_gain: float) -> nn.Sequential:
    layers, d = [], i
    for h in hidden:
        layers += [nn.Linear(d, h), nn.ELU()]
        d = h
    layers.append(nn.Linear(d, o))
    for layer in layers:
        if isinstance(layer, nn.Linear):
            nn.init.orthogonal_(layer.weight, gain=2**0.5)
            nn.init.zeros_(layer.bias)
    nn.init.orthogonal_(layers[-1].weight, gain=out_gain)
    return nn.Sequential(*layers)


class ActorCritic(nn.Module):
    def __init__(self, obs_dim: int, act_dim: int, hidden: tuple[int, ...] = (128, 128), init_log_std: float = -0.5):
        super().__init__()
        self.norm = RunningNorm(obs_dim)
        self.actor = mlp(obs_dim, hidden, act_dim, out_gain=0.01)
        self.critic = mlp(obs_dim, hidden, 1, out_gain=1.0)
        self.log_std = nn.Parameter(torch.full((act_dim,), init_log_std))

    def dist(self, x: torch.Tensor) -> torch.distributions.Normal:
        return torch.distributions.Normal(self.actor(x), self.log_std.exp().expand(x.shape[0], -1))

    def value(self, x: torch.Tensor) -> torch.Tensor:
        return self.critic(x).squeeze(-1)


class MLPPolicy:
    """Deterministic inference wrapper implementing the `Policy` protocol."""

    def __init__(self, model: ActorCritic, obs_key: str = "policy"):
        self.model, self.obs_key = model.eval(), obs_key

    @torch.no_grad()
    def act(self, obs: dict[str, torch.Tensor]) -> torch.Tensor:
        return self.model.actor(self.model.norm(obs[self.obs_key])).clamp(-1, 1)

    def reset(self, env_ids: torch.Tensor | None = None) -> None:
        pass

    def to(self, device: torch.device | str) -> MLPPolicy:
        self.model.to(device)
        return self

    def save(self, path: str | Path, **meta) -> None:
        m = self.model
        torch.save(
            {
                "state_dict": m.state_dict(),
                "obs_key": self.obs_key,
                "obs_dim": m.norm.mean.shape[0],
                "act_dim": m.log_std.shape[0],
                "hidden": [l.out_features for l in m.actor if isinstance(l, nn.Linear)][:-1],
                **meta,
            },
            path,
        )

    @classmethod
    def load(cls, path: str | Path, device: str | torch.device = "cpu") -> tuple[MLPPolicy, dict]:
        ckpt = torch.load(path, map_location=device, weights_only=False)
        model = ActorCritic(ckpt["obs_dim"], ckpt["act_dim"], tuple(ckpt["hidden"]))
        model.load_state_dict(ckpt["state_dict"])
        return cls(model.to(device), ckpt["obs_key"]), ckpt


def train(env: VecEnv, cfg: PPOConfig, log: Callable[[dict], None] = print) -> MLPPolicy:
    dev, n, t_len = env.device, env.num_envs, cfg.rollout_steps
    obs_dim, act_dim = env.spec.observations[cfg.obs_key].shape[0], env.spec.action_dim
    ac = ActorCritic(obs_dim, act_dim, cfg.hidden, cfg.init_log_std).to(dev)
    opt = torch.optim.Adam(ac.parameters(), lr=cfg.lr)

    buf_x = torch.zeros(t_len, n, obs_dim, device=dev)
    buf_a = torch.zeros(t_len, n, act_dim, device=dev)
    buf_logp, buf_r, buf_done, buf_v = (torch.zeros(t_len, n, device=dev) for _ in range(4))

    obs = env.reset()[cfg.obs_key]
    iters = max(1, cfg.total_steps // (t_len * n))
    start = time.perf_counter()
    for it in range(iters):
        ep_stats: dict[str, list[torch.Tensor]] = {}
        with torch.no_grad():
            for t in range(t_len):
                ac.norm.update(obs)
                x = ac.norm(obs)
                dist = ac.dist(x)
                a = dist.sample()
                v = ac.value(x)
                res = env.step(a)
                buf_x[t], buf_a[t], buf_logp[t], buf_v[t] = x, a, dist.log_prob(a).sum(-1), v
                buf_r[t] = res.reward + cfg.gamma * v * res.truncated.float()
                buf_done[t] = (res.terminated | res.truncated).float()
                for k, val in res.info.get("episode", {}).items():
                    ep_stats.setdefault(k, []).append(val.float())
                obs = res.obs[cfg.obs_key]

            next_v = ac.value(ac.norm(obs))
            adv = torch.zeros_like(buf_r)
            gae = torch.zeros(n, device=dev)
            for t in reversed(range(t_len)):
                nonterminal = 1.0 - buf_done[t]
                delta = buf_r[t] + cfg.gamma * next_v * nonterminal - buf_v[t]
                gae = delta + cfg.gamma * cfg.gae_lambda * nonterminal * gae
                adv[t] = gae
                next_v = buf_v[t]
            ret = adv + buf_v

        b = t_len * n
        fx, fa, flogp, fadv, fret = (z.reshape(b, *z.shape[2:]) for z in (buf_x, buf_a, buf_logp, adv, ret))
        mb = b // cfg.minibatches
        for _ in range(cfg.epochs):
            perm = torch.randperm(b, device=dev)
            for i in range(cfg.minibatches):
                idx = perm[i * mb : (i + 1) * mb]
                dist = ac.dist(fx[idx])
                ratio = (dist.log_prob(fa[idx]).sum(-1) - flogp[idx]).exp()
                a_mb = (fadv[idx] - fadv[idx].mean()) / (fadv[idx].std() + 1e-8)
                pg = -torch.min(ratio * a_mb, ratio.clamp(1 - cfg.clip, 1 + cfg.clip) * a_mb).mean()
                vf = (ac.value(fx[idx]) - fret[idx]).pow(2).mean()
                ent = dist.entropy().sum(-1).mean()
                loss = pg + cfg.vf_coef * vf - cfg.ent_coef * ent
                opt.zero_grad()
                loss.backward()
                nn.utils.clip_grad_norm_(ac.parameters(), cfg.max_grad_norm)
                opt.step()

        steps = (it + 1) * b
        row = {
            "iter": it + 1,
            "steps": steps,
            "sps": int(steps / (time.perf_counter() - start)),
            "pg_loss": round(pg.item(), 4),
            "vf_loss": round(vf.item(), 4),
            "std": round(ac.log_std.exp().mean().item(), 3),
        }
        row |= {f"ep_{k}": round(torch.cat(v).mean().item(), 3) for k, v in ep_stats.items()}
        log(row)
    return MLPPolicy(ac, cfg.obs_key)
