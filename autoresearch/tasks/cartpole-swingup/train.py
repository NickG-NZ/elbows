"""Train a cart-pole swing-up policy within AR_TIME_BUDGET seconds. THIS is the file agents edit.

Baseline: cross-entropy method (CEM) over the weights of a small tanh MLP, with the whole population
simulated in one batch. Contract: save a TorchScript module to $AR_OUT_DIR/policy.pt that maps
obs [n, 5] -> action [n, 1] in [-1, 1]. Anything else (algorithm, network, sim usage) is fair game.
"""

from __future__ import annotations

import os
import time

import torch
from torch import nn

from sim import ACT_DIM, OBS_DIM, CartPole, rollout

BUDGET = float(os.environ.get("AR_TIME_BUDGET", "60"))
SEED = int(os.environ.get("AR_SEED", "0"))
OUT_DIR = os.environ.get("AR_OUT_DIR", "artifacts")

HIDDEN = 16
POP = 64
ELITE = 8
EPISODES = 8  # per population member
INIT_STD = 0.5
STD_DECAY = 0.98
MIN_STD = 0.02
TRAIN_PARAM_NOISE = 0.10


class MLP(nn.Module):
    def __init__(self, hidden: int = HIDDEN):
        super().__init__()
        self.l1 = nn.Linear(OBS_DIM, hidden)
        self.l2 = nn.Linear(hidden, ACT_DIM)

    def forward(self, obs: torch.Tensor) -> torch.Tensor:
        return torch.tanh(self.l2(torch.tanh(self.l1(obs))))


SHAPES = [(OBS_DIM, HIDDEN), (HIDDEN,), (HIDDEN, ACT_DIM), (ACT_DIM,)]
N_PARAMS = sum(torch.Size(s).numel() for s in SHAPES)


def unflatten(theta: torch.Tensor) -> list[torch.Tensor]:
    """theta [P, N_PARAMS] -> per-member weight tensors with a leading population dim."""
    out, i = [], 0
    for s in SHAPES:
        k = torch.Size(s).numel()
        out.append(theta[:, i : i + k].reshape(theta.shape[0], *s))
        i += k
    return out


def batched_policy(theta: torch.Tensor, episodes: int):
    w1, b1, w2, b2 = unflatten(theta)

    def act(obs: torch.Tensor) -> torch.Tensor:  # obs [P*E, 5]
        o = obs.reshape(theta.shape[0], episodes, OBS_DIM)
        h = torch.tanh(torch.bmm(o, w1) + b1[:, None])
        return torch.tanh(torch.bmm(h, w2) + b2[:, None]).reshape(-1)

    return act


def main() -> None:
    torch.manual_seed(SEED)
    torch.set_num_threads(min(4, os.cpu_count() or 1))
    g = torch.Generator().manual_seed(SEED)
    start = time.time()

    mean = torch.zeros(N_PARAMS)
    std = torch.full((N_PARAMS,), INIT_STD)
    best_theta, best_score = mean.clone(), -float("inf")
    gen = 0
    while time.time() - start < BUDGET * 0.95:
        theta = mean + std * torch.randn(POP, N_PARAMS, generator=g)
        env = CartPole(POP * EPISODES, g, param_noise=TRAIN_PARAM_NOISE)
        with torch.no_grad():
            returns = rollout(batched_policy(theta, EPISODES), env).reshape(POP, EPISODES).mean(1)
        elite = returns.topk(ELITE).indices
        mean = theta[elite].mean(0)
        std = (theta[elite].std(0) * 0.5 + std * 0.5).clamp(min=MIN_STD) * STD_DECAY
        if returns.max() > best_score:
            best_score, best_theta = float(returns.max()), theta[returns.argmax()].clone()
        gen += 1
        if gen % 10 == 0:
            print(f"gen {gen:4d}  t={time.time() - start:5.1f}s  mean_ret={returns.mean():7.2f}  best={best_score:7.2f}")

    # Export the CEM mean (more robust than the single luckiest sample) as TorchScript.
    policy = MLP()
    with torch.no_grad():
        w1, b1, w2, b2 = (t[0] for t in unflatten(mean[None]))
        policy.l1.weight.copy_(w1.T)
        policy.l1.bias.copy_(b1)
        policy.l2.weight.copy_(w2.T)
        policy.l2.bias.copy_(b2)
    os.makedirs(OUT_DIR, exist_ok=True)
    torch.jit.script(policy).save(os.path.join(OUT_DIR, "policy.pt"))
    print(f"done: {gen} generations in {time.time() - start:.1f}s, best sample return {best_score:.2f}")


if __name__ == "__main__":
    main()
