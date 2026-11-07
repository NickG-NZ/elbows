"""`elbows-learn` command line: list / train <recipe> / eval <checkpoint>."""

from __future__ import annotations

import argparse
import dataclasses
import json
from pathlib import Path

import torch
from elbows_sim.backends import BACKENDS
from elbows_sim.evaluate import evaluate, save_video

from elbows_sim import make

from .recipes import RECIPES, Recipe
from .rl import MLPPolicy, train


def resolve(a: argparse.Namespace) -> Recipe:
    """The named recipe with any command-line overrides applied."""
    r = RECIPES[a.recipe]
    overrides = {k: v for k, v in {"backend": a.backend, "num_envs": a.num_envs}.items() if v is not None}
    ppo = dataclasses.replace(r.ppo, total_steps=a.steps) if a.steps else r.ppo
    return dataclasses.replace(r, ppo=ppo, **overrides)


def cmd_list(_: argparse.Namespace) -> None:
    for r in RECIPES.values():
        print(f"{r.name:<20} task={r.task:<10} {r.description}")


def cmd_train(a: argparse.Namespace) -> None:
    r = resolve(a)
    out = Path(a.out or f"runs/{r.name}-{r.backend}-s{a.seed}")
    out.mkdir(parents=True, exist_ok=True)
    (out / "recipe.json").write_text(json.dumps(r.to_dict(), indent=2))
    torch.manual_seed(a.seed)
    env = make(r.task, backend=r.backend, num_envs=r.num_envs, device=a.device, seed=a.seed, **r.task_kwargs)
    log_f = (out / "train.jsonl").open("w")

    def log(row: dict) -> None:
        log_f.write(json.dumps(row) + "\n")
        log_f.flush()
        if row["iter"] % a.log_every == 0:
            print(" ".join(f"{k}={v}" for k, v in row.items()), flush=True)

    policy = train(env, r.ppo, log)
    env.close()
    policy.save(out / "policy.pt", recipe=r.to_dict())
    print(f"saved {out / 'policy.pt'}")
    if r.eval_episodes:
        run_eval(
            out / "policy.pt",
            backend=r.backend,
            episodes=r.eval_episodes,
            num_envs=min(r.num_envs, r.eval_episodes),
            device=a.device,
            seed=a.seed,
            video=out / "eval.mp4",
        )


def run_eval(
    checkpoint: Path,
    backend: str | None,
    episodes: int,
    num_envs: int,
    device: str | None,
    seed: int,
    video: Path | None,
) -> None:
    policy, ckpt = MLPPolicy.load(checkpoint)
    r = ckpt["recipe"]
    env = make(
        r["task"], backend=backend or r["backend"], num_envs=num_envs, device=device, seed=seed, **r["task_kwargs"]
    )
    policy.to(env.device)
    stats, frames = evaluate(env, policy, episodes, video_env=0 if video else None)
    print("eval", json.dumps({k: round(v, 4) for k, v in stats.items()}))
    if video and frames:
        save_video(frames, video, fps=1 / env.spec.control_dt)
        print(f"saved {video}")
    env.close()


def cmd_eval(a: argparse.Namespace) -> None:
    run_eval(
        Path(a.checkpoint), a.backend, a.episodes, a.num_envs, a.device, a.seed, Path(a.video) if a.video else None
    )


def main() -> None:
    p = argparse.ArgumentParser(prog="elbows-learn")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list", help="list recipes").set_defaults(fn=cmd_list)

    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--backend", default=None, choices=sorted(BACKENDS), help="override the recipe's backend")
    common.add_argument("--device", default=None)
    common.add_argument("--seed", type=int, default=0)

    t = sub.add_parser("train", parents=[common], help="train with a recipe")
    t.add_argument("recipe", choices=sorted(RECIPES))
    t.add_argument("--num-envs", type=int, default=None, help="override the recipe's num_envs")
    t.add_argument("--steps", type=int, default=None, help="override the recipe's total_steps")
    t.add_argument("--out", default=None, help="default: runs/<recipe>-<backend>-s<seed>")
    t.add_argument("--log-every", type=int, default=10)
    t.set_defaults(fn=cmd_train)

    e = sub.add_parser("eval", parents=[common], help="evaluate a checkpoint, optionally with video")
    e.add_argument("checkpoint")
    e.add_argument("--num-envs", type=int, default=16)
    e.add_argument("--episodes", type=int, default=32)
    e.add_argument("--video", default=None)
    e.set_defaults(fn=cmd_eval)

    a = p.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
