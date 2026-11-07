"""Backend-agnostic policy evaluation and video capture."""

from __future__ import annotations

from pathlib import Path

import torch

from .core.env import VecEnv
from .policies import Policy


@torch.no_grad()
def evaluate(
    env: VecEnv,
    policy: Policy,
    episodes: int,
    seed: int = 1234,
    video_env: int | None = None,
    camera: str | None = None,
    size: int = 320,
) -> tuple[dict[str, float], list[torch.Tensor]]:
    """Run until `episodes` episodes finish. Returns mean episode stats and (optional) frames of one env."""
    obs = env.reset(seed=seed)
    policy.reset()
    stats: dict[str, list[torch.Tensor]] = {}
    frames: list[torch.Tensor] = []
    finished = 0
    vid_ids = None if video_env is None else torch.tensor([video_env], device=env.device)
    while finished < episodes:
        if vid_ids is not None:
            frames.append(env.render(camera, vid_ids, width=size, height=size)[0].cpu())
        res = env.step(policy.act(obs))
        obs = res.obs
        ep = res.info.get("episode")
        if ep:
            finished += len(ep["return"])
            for k, v in ep.items():
                stats.setdefault(k, []).append(v.float().cpu())
            done_ids = (res.terminated | res.truncated).nonzero().squeeze(-1)
            policy.reset(done_ids)
            if vid_ids is not None and bool((done_ids == video_env).any()):
                vid_ids = None  # one full episode is enough
    return {k: torch.cat(v)[:episodes].mean().item() for k, v in stats.items()}, frames


def save_video(frames: list[torch.Tensor], path: str | Path, fps: float) -> None:
    import imageio.v2 as imageio

    Path(path).parent.mkdir(parents=True, exist_ok=True)
    imageio.mimsave(path, [f.numpy() for f in frames], fps=fps)
