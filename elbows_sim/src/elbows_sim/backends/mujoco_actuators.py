"""Translate simulator-neutral actuators (`elbows_sim.core.actuators`) into MuJoCo joints and actuators."""

from __future__ import annotations

import mujoco

from ..core.actuators import Actuator, DCMotor, PositionServo


def add_actuator(
    spec: mujoco.MjSpec, joint: mujoco.MjsJoint, act: Actuator, hard_stop: tuple[float, float] | None = None
) -> mujoco.MjsActuator:
    """Set the joint's drivetrain dynamics and add one actuator driving it. With `hard_stop`, the joint
    may travel past its (URDF) command range to the physical stop."""
    if not joint.range[0] < joint.range[1]:
        raise ValueError(f"joint {joint.name!r} needs finite limits")
    joint.armature = act.armature
    joint.frictionloss = act.frictionloss
    joint.damping = [act.damping, 0.0, 0.0]  # MuJoCo 4 takes damping as polynomial coefficients
    a = spec.add_actuator(name=joint.name, target=joint.name, trntype=mujoco.mjtTrn.mjTRN_JOINT)

    match act:
        case PositionServo():
            a.set_to_position(kp=act.kp, kv=act.kv)
            a.ctrllimited, a.ctrlrange = True, joint.range.copy()
        case DCMotor():
            a.set_to_motor()
            a.ctrllimited, a.ctrlrange = True, [-act.stall_torque, act.stall_torque]
        case _:
            raise NotImplementedError(f"MuJoCo backend can't model {type(act).__name__} actuators yet")

    if isinstance(act, DCMotor):
        # Torque clip at stall plus back-EMF as (implicitly integrated) joint damping: the linear
        # torque-speed curve.
        a.forcelimited, a.forcerange = True, [-act.stall_torque, act.stall_torque]
        joint.damping = [act.back_emf_damping + act.damping, 0.0, 0.0]

    if hard_stop is not None:
        lo, hi = hard_stop
        if not (lo <= joint.range[0] and joint.range[1] <= hi):
            raise ValueError(f"hard stop {hard_stop} of {joint.name!r} must contain its command range {joint.range}")
        joint.range = [lo, hi]
    return a
