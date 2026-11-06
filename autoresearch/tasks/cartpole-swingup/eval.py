"""Evaluate artifacts/policy.pt. FROZEN: agents must not edit this file.

Visible metric (what the agent optimises):
  val_return  mean return over 128 fixed episodes with +-10% physical-parameter noise.
Hidden metric (written to $AR_HIDDEN_METRICS_FILE, never printed):
  holdout_return  256 episodes drawn from a salted seed with +-30% parameter noise + friction.
A policy that only works on the val seeds, or only on nominal physics, shows up as a gap.
"""

from __future__ import annotations

import json
import os
import zlib

import torch

from sim import CartPole, rollout

MAX_RETURN = 500.0


def load_policy(path: str):
    policy = torch.jit.load(path, map_location="cpu")
    policy.eval()

    @torch.no_grad()
    def act(obs: torch.Tensor) -> torch.Tensor:
        return policy(obs).float()

    return act


def score(act, n: int, seed: int, noise: float) -> float:
    g = torch.Generator().manual_seed(seed)
    env = CartPole(n, g, param_noise=noise)
    with torch.no_grad():
        ret = rollout(act, env)
    ret = torch.nan_to_num(ret, nan=0.0).clamp(max=MAX_RETURN)
    return float(ret.mean())


def main() -> None:
    torch.set_num_threads(min(4, os.cpu_count() or 1))
    out_dir = os.environ.get("AR_OUT_DIR", "artifacts")
    act = load_policy(os.path.join(out_dir, "policy.pt"))

    print(f"METRIC val_return={score(act, 128, seed=1234, noise=0.10):.4f}")

    hidden_file = os.environ.get("AR_HIDDEN_METRICS_FILE")
    if hidden_file:
        salt = os.environ.get("AR_HOLDOUT_SALT", "")
        seed = zlib.crc32(salt.encode()) & 0x7FFFFFFF
        with open(hidden_file, "w") as f:
            json.dump({"holdout_return": score(act, 256, seed=seed, noise=0.30)}, f)


if __name__ == "__main__":
    main()
