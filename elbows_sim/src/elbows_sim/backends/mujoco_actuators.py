"""Translate simulator-neutral `ActuatorModel`s into MuJoCo joints and actuators."""

from __future__ import annotations

import mujoco

from ..core.spec import ActuatorModel


def add_actuator(
    spec: mujoco.MjSpec, joint: mujoco.MjsJoint, model: ActuatorModel, hard_stop: tuple[float, float] | None = None
) -> mujoco.MjsActuator:
    """Set the joint's servo-side dynamics and add one actuator driving it. Controls span the joint's
    (URDF) range; with `hard_stop`, the joint itself is allowed to travel to the physical stop."""
    if model.control != "position":
        raise NotImplementedError(f"MuJoCo backend has no translation for control={model.control!r}")
    if not joint.range[0] < joint.range[1]:
        raise ValueError(f"joint {joint.name!r} needs finite limits so actions can map onto them")

    # Back-EMF plus gearbox friction as (implicitly integrated) joint damping: together with the force
    # clip below this is the linear torque-speed curve. MuJoCo 4 takes damping as coefficients.
    joint.damping = [model.back_emf_damping + model.damping, 0.0, 0.0]
    joint.armature = model.armature
    joint.frictionloss = model.frictionloss

    a = spec.add_actuator(name=joint.name, target=joint.name, trntype=mujoco.mjtTrn.mjTRN_JOINT)
    a.set_to_position(kp=model.kp, kv=model.kv)
    a.forcelimited, a.forcerange = True, [-model.stall_torque, model.stall_torque]
    a.ctrllimited, a.ctrlrange = True, joint.range.copy()
    if hard_stop is not None:
        lo, hi = hard_stop
        if not (lo <= joint.range[0] and joint.range[1] <= hi):
            raise ValueError(f"hard stop {hard_stop} of {joint.name!r} must contain its command range {joint.range}")
        joint.range = [lo, hi]
    return a
