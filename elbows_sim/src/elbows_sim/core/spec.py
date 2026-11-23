"""Backend-neutral descriptions of scenes, observations and environments."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

import torch


@dataclass(frozen=True)
class TensorSpec:
    """Shape and dtype of one observation entry, per environment (no batch dim)."""

    shape: tuple[int, ...]
    dtype: torch.dtype = torch.float32


@dataclass(frozen=True)
class ActuatorModel:
    """A joint servo, described independently of any simulator. Stored as TOML in `assets/actuators/`
    (read by `elbows_sim.formats.actuator`) and translated by each backend.

    control="position" models a voltage-driven DC servo with a position loop:

        drive  = clip(kp * (target - q) - kv * qd, -stall_torque, +stall_torque)
        torque = drive - (stall_torque / no_load_speed + damping) * qd - frictionloss * sign(qd)

    The back-EMF term (stall_torque / no_load_speed) gives the linear torque-speed curve: full
    torque at rest, none at `no_load_speed`, so a saturated, unloaded joint tops out there. Actions
    in [-1, 1] map onto the joint's command range, so an action is a target joint position.
    """

    name: str
    kp: float  # N·m/rad
    stall_torque: float  # N·m, at the rated supply voltage
    no_load_speed: float  # rad/s, at the rated supply voltage
    kv: float = 0.0  # N·m·s/rad, extra derivative gain in the position loop
    damping: float = 0.0  # N·m·s/rad, viscous friction on top of back-EMF (gearbox)
    armature: float = 0.0  # kg·m², reflected rotor inertia
    frictionloss: float = 0.0  # N·m, dry friction
    control: str = "position"
    source: str = ""  # where the numbers come from

    @property
    def back_emf_damping(self) -> float:
        return self.stall_torque / self.no_load_speed


@dataclass(frozen=True)
class RobotSpec:
    """A robot loaded from a URDF and placed into a scene at the site named `mount`.

    Backends add one actuator per joint in `actuators` (joint name -> servo model).

    Limits: the URDF's joint limits are the *command* range (actions map onto them, a software
    safety margin). `hard_stops` (joint -> (lower, upper) rad) are the physical end stops where the
    joint actually stops, and must contain the command range. Joints without one stop at their URDF
    limits.
    """

    urdf: Path
    actuators: dict[str, ActuatorModel]
    hard_stops: dict[str, tuple[float, float]] = field(default_factory=dict)
    mount: str = "robot_mount"
    self_collision: bool = False


@dataclass(frozen=True)
class SceneSpec:
    """Everything a backend needs to build a scene, without saying how.

    `assets` maps an asset format to a file, e.g. {"mjcf": ..., "usd": ...}. Each backend picks the
    format it can load. When adding Isaac, either author a USD next to the MJCF or convert the MJCF
    with Isaac's importer, and keep joint/site/camera names identical so tasks work unchanged.

    With `robot` set, the "mjcf" asset is the surrounding scene (floor, cameras, markers, a mount
    site) and the robot comes from its URDF. See `elbows_sim.robots`.
    """

    name: str
    assets: dict[str, Path]
    actuated_joints: tuple[str, ...]  # order defines the action vector
    frames: tuple[str, ...] = ()  # named points tasks read world positions of (MJCF sites/bodies)
    markers: tuple[str, ...] = ()  # kinematic, non-colliding bodies tasks can move (targets, goals)
    cameras: tuple[str, ...] = ()
    physics_dt: float = 0.005
    decimation: int = 4  # physics steps per control step
    robot: RobotSpec | None = None

    @property
    def control_dt(self) -> float:
        return self.physics_dt * self.decimation


@dataclass(frozen=True)
class EnvSpec:
    """What a policy or learner may rely on. Actions are always normalised to [-1, 1]."""

    observations: dict[str, TensorSpec]
    action_dim: int
    control_dt: float
    max_episode_steps: int
    instruction: str = ""  # natural-language task description, for language-conditioned policies
    metadata: dict[str, object] = field(default_factory=dict)
