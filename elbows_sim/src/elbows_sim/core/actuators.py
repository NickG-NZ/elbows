"""Actuator models, described independently of any simulator.

One dataclass per physical kind of actuator. Each backend translates every kind it supports
(`backends/mujoco_actuators.py`) and raises NotImplementedError for the rest. In files, a table's
`type` key picks the kind (see `elbows_sim.formats.actuators`).

Kinds so far:
    DCMotor        torque-commanded DC motor (brushed, or BLDC under FOC) after its gearbox
    PositionServo  DCMotor with a position loop: hobby and bus servos (STS3215, Dynamixel)

To add one (hydraulic cylinder, series-elastic, ...): subclass Actuator, give it a `type`, list it in
TYPES, and add a case to each backend's translation.
"""

from __future__ import annotations

from dataclasses import dataclass, fields
from typing import ClassVar


@dataclass(frozen=True, kw_only=True)
class Actuator:
    """Drivetrain properties shared by every kind, as seen at the joint."""

    type: ClassVar[str]
    armature: float = 0.0  # kg·m², reflected rotor inertia
    damping: float = 0.0  # N·m·s/rad, viscous friction (gearbox)
    frictionloss: float = 0.0  # N·m, dry friction
    source: str = ""  # where the numbers come from

    def __post_init__(self) -> None:
        for f in fields(self):
            v = getattr(self, f.name)
            if isinstance(v, (int, float)) and v < 0:
                raise ValueError(f"{type(self).__name__}.{f.name} must be >= 0, got {v}")


@dataclass(frozen=True, kw_only=True)
class DCMotor(Actuator):
    """A DC motor at a fixed supply voltage; the action is a torque command.

        torque = clip(command, -stall_torque, +stall_torque) - (back_emf_damping + damping) * qd

    The back-EMF term gives the linear torque-speed curve: full torque at rest, none at
    `no_load_speed`, so a saturated, unloaded joint tops out there.
    """

    type: ClassVar[str] = "dc_motor"
    stall_torque: float  # N·m
    no_load_speed: float  # rad/s

    def __post_init__(self) -> None:
        super().__post_init__()
        if self.stall_torque <= 0 or self.no_load_speed <= 0:
            raise ValueError("stall_torque and no_load_speed must be > 0")

    @property
    def back_emf_damping(self) -> float:
        return self.stall_torque / self.no_load_speed


@dataclass(frozen=True, kw_only=True)
class PositionServo(DCMotor):
    """A DCMotor driven by a position loop; the action is a target position across the joint's
    command range.

        command = kp * (target - q) - kv * qd
    """

    type: ClassVar[str] = "position_servo"
    kp: float  # N·m/rad
    kv: float = 0.0  # N·m·s/rad, derivative gain of the loop


TYPES: dict[str, type[Actuator]] = {cls.type: cls for cls in (DCMotor, PositionServo)}
