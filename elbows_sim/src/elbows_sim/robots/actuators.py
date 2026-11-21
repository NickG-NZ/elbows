"""Servo models shared by every robot that uses them. Add a servo once here, then map joints to it."""

from __future__ import annotations

from ..core.spec import ActuatorModel

# Feetech STS3215 (SO-100/SO-101 joints). Values from TheRobotStudio's SO-101 MJCF
# (Simulation/SO101/joints_properties.xml), which adapts them from the Open Duck Mini project.
# Not yet identified on our own servos.
STS3215 = ActuatorModel(
    name="sts3215", kp=17.8, kv=0.0, force_limit=3.35, damping=0.60, armature=0.028, frictionloss=0.052
)

ACTUATORS: dict[str, ActuatorModel] = {a.name: a for a in (STS3215,)}
