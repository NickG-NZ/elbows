"""Translate simulator-neutral `ActuatorModel`s into MuJoCo joints and actuators."""

from __future__ import annotations

import mujoco

from ..core.spec import ActuatorModel


def add_actuator(spec: mujoco.MjSpec, joint: mujoco.MjsJoint, model: ActuatorModel) -> mujoco.MjsActuator:
    """Set the joint's servo-side dynamics and add one actuator driving it. Controls span the joint range."""
    if model.control != "position":
        raise NotImplementedError(f"MuJoCo backend has no translation for control={model.control!r}")
    if not joint.range[0] < joint.range[1]:
        raise ValueError(f"joint {joint.name!r} needs finite limits so actions can map onto them")

    joint.damping = [model.damping, 0.0, 0.0]  # MuJoCo 4 takes damping as polynomial coefficients
    joint.armature = model.armature
    joint.frictionloss = model.frictionloss

    a = spec.add_actuator(name=joint.name, target=joint.name, trntype=mujoco.mjtTrn.mjTRN_JOINT)
    a.set_to_position(kp=model.kp, kv=model.kv)
    a.forcelimited, a.forcerange = True, [-model.force_limit, model.force_limit]
    a.ctrllimited, a.ctrlrange = True, joint.range
    return a
