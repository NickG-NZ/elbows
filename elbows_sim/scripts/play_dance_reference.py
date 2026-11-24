"""Render the so101_dance reference trajectory: the arm posed kinematically on the reference at every
control step (no policy, no physics). One clip per sampled dance, each with its own tempo.

    uv run python elbows_sim/scripts/play_dance_reference.py --out runs/dance_reference.mp4
    uv run python elbows_sim/scripts/play_dance_reference.py --urdf ~/so101_6dof.urdf --dances 2

Also reports reference poses that collide (self or floor), which the trained policy can't follow
without ending the episode.
"""

from __future__ import annotations

import argparse
import math

import torch
from elbows_sim.evaluate import save_video
from elbows_sim.tasks.so101_dance import EE_AXIS, EE_FRAME

import elbows_sim


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--out", default="runs/dance_reference.mp4")
    p.add_argument("--urdf", default=None)
    p.add_argument("--dances", type=int, default=3, help="number of sampled dances (episodes)")
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--camera", default="front")
    p.add_argument("--size", type=int, nargs=2, default=(640, 480), metavar=("W", "H"))
    a = p.parse_args()

    env = elbows_sim.make("so101_dance", num_envs=1, **({"urdf": a.urdf} if a.urdf else {}))
    task, sim = env.task, env.sim
    ids = torch.zeros(1, dtype=torch.long, device=env.device)
    dt = sim.control_dt
    frames = []
    for k in range(a.dances):
        env.reset(seed=a.seed + k)
        bpm = task.beat_hz[0].item() * 60
        contacts, tilt = 0, []
        for _ in range(task.max_episode_steps):
            q = task.reference(task.t)
            qd = (task.reference(task.t + dt) - q) / dt
            sim.set_joint_state(ids, q, qd)
            contacts += int(sim.robot_contacts().sum() > 0)
            tilt.append(math.degrees(math.acos(sim.frame_rot(EE_FRAME)[0, 2, EE_AXIS].clamp(-1, 1).item())))
            frames.append(sim.render(a.camera, ids, *a.size)[0].cpu())
            task.t += dt
        print(
            f"dance {k}: {bpm:.0f} BPM, {task.max_episode_steps * dt:.0f} s, "
            f"gripper tilt mean {sum(tilt) / len(tilt):.0f} deg (max {max(tilt):.0f}; wrist_flex is left to the policy), "
            f"colliding frames {contacts}"
        )
    save_video(frames, a.out, fps=1 / dt)
    print(f"saved {a.out}")
    env.close()


if __name__ == "__main__":
    main()
