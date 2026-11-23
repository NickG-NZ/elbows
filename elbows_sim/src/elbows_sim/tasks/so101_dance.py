"""SO-101 dance: follow a beat-synchronised joint reference while keeping the gripper pointing up.

Each episode samples a tempo and, for every *danced* joint, a sum of sinusoids at fractions of the
beat frequency (random amplitudes and phases) around an upright home pose. Joints without a
reference (by default `wrist_flex`) are left to the policy, which is rewarded for keeping the
gripper's axis vertical. So PPO has to learn the posture, not just copy the reference.

Safety: any self-collision or floor contact ends the episode with a penalty.

Works with any SO-101 URDF: `make("so101_dance", urdf=...)`. Joint roles go by name, so a forearm
roll called `elbow_rotate` is danced automatically; other added joints are left free.
"""

from __future__ import annotations

import math
from pathlib import Path

import torch

from ..core.sim import Sim
from ..core.spec import SceneSpec, TensorSpec
from ..core.task import Task
from ..formats.urdf import urdf_joints
from ..robots.so101 import SCENE, so101_robot

# Upright home pose (rad): forearm angled up, gripper vertical (shoulder_lift + elbow_flex + wrist_flex
# = -pi/2 on the stock arm). Unlisted joints start at 0.
HOME = {"elbow_flex": -0.8, "wrist_flex": -0.77, "gripper": 0.3}

# Danced joints: (max amplitude in rad, beat fractions to draw sinusoid frequencies from). Amplitudes
# and frequencies are capped so peak speeds stay a few rad/s, inside what an STS3215 can do.
DANCE = {
    "shoulder_pan": (0.6, (0.25, 0.5)),
    "shoulder_lift": (0.3, (0.25, 0.5)),
    "elbow_flex": (0.4, (0.25, 0.5)),
    "elbow_rotate": (0.6, (0.25, 0.5)),  # forearm roll on modified arms
    "wrist_roll": (0.8, (0.25, 0.5)),
    "gripper": (0.4, (0.5,)),
}

EE_FRAME = "gripper_frame_link"
EE_AXIS = 2  # the gripper frame's z-axis points out of the gripper
FUTURE_S = (0.1, 0.2)  # reference look-ahead given to the policy, seconds


class So101Dance(Task):
    instruction = "dance to the beat with the gripper pointing up"

    def __init__(
        self,
        urdf: str | Path | None = None,
        actuators: str | Path | None = None,  # actuator file; default: stock STS3215s
        max_episode_steps: int = 250,  # 5 s at 50 Hz
        bpm: tuple[float, float] = (90.0, 130.0),
        harmonics: int = 2,
        upright_weight: float = 0.4,
        contact_penalty: float = 10.0,
        self_collision: bool = True,
    ):
        robot = so101_robot(urdf, actuators, self_collision=self_collision)
        joints = urdf_joints(robot.urdf)
        names = tuple(j.name for j in joints)
        self.scene = SceneSpec(
            name=f"so101:{robot.urdf.stem}",
            assets={"mjcf": SCENE},
            actuated_joints=names,
            frames=(EE_FRAME,),
            cameras=("front", "top"),
            physics_dt=0.005,
            decimation=4,  # 50 Hz control
            robot=robot,
        )
        self.lo = torch.tensor([j.lower for j in joints])
        self.hi = torch.tensor([j.upper for j in joints])
        self.home = torch.tensor([HOME.get(n, 0.0) for n in names]).clamp(self.lo, self.hi)
        self.danced = torch.tensor([n in DANCE for n in names])
        self.max_amp = torch.tensor([DANCE[n][0] if n in DANCE else 0.0 for n in names])
        self.fractions = [DANCE[n][1] if n in DANCE else (0.0,) for n in names]
        self.max_episode_steps = max_episode_steps
        self.bpm = bpm
        self.harmonics = harmonics
        self.upright_weight = upright_weight
        self.contact_penalty = contact_penalty

    @property
    def num_joints(self) -> int:
        return len(self.scene.actuated_joints)

    def bind(self, num_envs: int, device: torch.device) -> None:
        super().bind(num_envs, device)
        n, h = self.num_joints, self.harmonics
        for k in ("lo", "hi", "home", "danced", "max_amp"):
            setattr(self, k, getattr(self, k).to(device))
        self._mid, self._half = (self.hi + self.lo) / 2, (self.hi - self.lo) / 2
        self.t = torch.zeros(num_envs, device=device)
        self.beat_hz = torch.ones(num_envs, device=device)
        self.amp = torch.zeros(num_envs, n, h, device=device)
        self.freq = torch.zeros(num_envs, n, h, device=device)
        self.phase = torch.zeros(num_envs, n, h, device=device)
        self.prev_action = torch.zeros(num_envs, n, device=device)
        self._sums = torch.zeros(num_envs, 3, device=device)  # tracking error, upright, steps
        self._contact = torch.zeros(num_envs, dtype=torch.bool, device=device)

    # --- reference -------------------------------------------------------------------------------
    def reference(self, t: torch.Tensor) -> torch.Tensor:
        """[N, joints] reference joint positions at times t [N] (seconds)."""
        waves = (self.amp * torch.sin(2 * math.pi * self.freq * t[:, None, None] + self.phase)).sum(-1)
        return (self.home + waves).clamp(self.lo, self.hi)

    def _sample(self, ids: torch.Tensor, rng: torch.Generator) -> None:
        n, h, k = self.num_joints, self.harmonics, len(ids)
        u = lambda *s: torch.rand(*s, generator=rng).to(self.device)
        lo, hi = self.bpm
        self.beat_hz[ids] = (lo + (hi - lo) * u(k)) / 60
        # Split each joint's amplitude budget across harmonics so the sum never exceeds it.
        w = u(k, n, h)
        self.amp[ids] = self.max_amp[:, None] * w / w.sum(-1, keepdim=True).clamp_min(1e-6) * u(k, n, 1)
        choice = torch.stack(
            [
                torch.tensor(self.fractions[j])[torch.randint(len(self.fractions[j]), (k, h), generator=rng)]
                for j in range(n)
            ],
            dim=1,
        ).to(self.device)
        self.freq[ids] = choice * self.beat_hz[ids, None, None]
        self.phase[ids] = u(k, n, h) * 2 * math.pi

    # --- Task ------------------------------------------------------------------------------------
    def observation_spec(self) -> dict[str, TensorSpec]:
        n = self.num_joints
        return {"policy": TensorSpec((3 * n + len(FUTURE_S) * n + 2 + 3,))}

    def reset(self, sim: Sim, env_ids: torch.Tensor, rng: torch.Generator) -> None:
        sim.reset(env_ids)
        self._sample(env_ids, rng)
        self.t[env_ids] = 0.0
        q0 = self.reference(self.t)[env_ids]
        sim.set_joint_state(env_ids, q0, torch.zeros_like(q0))
        self.prev_action[env_ids] = self._normalise(q0)
        self._sums[env_ids] = 0.0
        self._contact[env_ids] = False

    def advance(self, sim: Sim) -> None:
        self.t += sim.control_dt
        # Safety: any self-collision or contact with the floor/scene ends the episode.
        self._contact = sim.robot_contacts().sum(-1) > 0

    def terminated(self, sim: Sim) -> torch.Tensor:
        return self._contact.clone()

    def _normalise(self, q: torch.Tensor) -> torch.Tensor:
        return (q - self._mid) / self._half

    def _upright(self, sim: Sim) -> torch.Tensor:
        """Cosine between the gripper axis and world up, in [-1, 1]."""
        return sim.frame_rot(EE_FRAME)[:, 2, EE_AXIS]

    def observe(self, sim: Sim) -> dict[str, torch.Tensor]:
        q = self._normalise(sim.joint_pos())
        ref = self._normalise(self.reference(self.t))
        future = [self._normalise(self.reference(self.t + dt)) - q for dt in FUTURE_S]
        beat = 2 * math.pi * self.beat_hz * self.t
        axis = sim.frame_rot(EE_FRAME)[:, :, EE_AXIS]
        obs = [q, 0.1 * sim.joint_vel(), (ref - q) * self.danced, *[f * self.danced for f in future]]
        return {"policy": torch.cat([*obs, beat.sin()[:, None], beat.cos()[:, None], axis], dim=-1)}

    def _tracking_error(self, sim: Sim) -> torch.Tensor:
        err = (sim.joint_pos() - self.reference(self.t)) * self.danced
        return torch.sqrt((err**2).sum(-1) / self.danced.sum())  # RMS over danced joints, rad

    def reward(self, sim: Sim, action: torch.Tensor) -> torch.Tensor:
        err, up = self._tracking_error(sim), self._upright(sim)
        track = torch.exp(-((err / 0.15) ** 2))
        upright = torch.exp(-(1 - up) / 0.05)  # 1 when vertical, ~0.14 at 25 degrees off
        smooth = ((action - self.prev_action) ** 2).sum(-1)
        self.prev_action = action.clone()
        self._sums += torch.stack([err, up, torch.ones_like(err)], dim=-1)
        w = self.upright_weight
        r = (1 - w) * track + w * upright - 0.05 * smooth - 0.001 * (sim.joint_vel() ** 2).sum(-1)
        return r - self.contact_penalty * self._contact.float()

    def metrics(self, sim: Sim) -> dict[str, torch.Tensor]:
        steps = self._sums[:, 2].clamp_min(1)
        upright_cos = self._sums[:, 1] / steps
        return {
            "tracking_rms": self._sums[:, 0] / steps,  # rad, episode mean
            "contact": self._contact.float(),  # 1 if the episode ended in a collision
            "upright_deg": torch.rad2deg(torch.arccos(upright_cos.clamp(-1, 1))),  # mean tilt from vertical
        }
