"""Read a robot's actuator file (TOML): which actuator drives each joint.

[default]                 # every joint, unless listed below
type = "position_servo"   # a kind from elbows_sim.core.actuators.TYPES; other keys are its fields
stall_torque = 2.942
...

[joints.gripper]          # replaces [default] for this joint
type = "position_servo"
...
"""

from __future__ import annotations

import tomllib
from pathlib import Path

from ..core.actuators import TYPES, Actuator


def load_actuators(path: str | Path, joints: list[str]) -> dict[str, Actuator]:
    """Joint name -> Actuator for every joint in `joints`."""
    path = Path(path)
    with path.open("rb") as f:
        data = tomllib.load(f)
    unknown = set(data) - {"default", "joints"}
    if unknown:
        raise ValueError(f"{path}: unknown tables {sorted(unknown)}; expected [default] and [joints.<name>]")
    per_joint = data.get("joints", {})
    missing = set(per_joint) - set(joints)
    if missing:
        raise ValueError(f"{path}: actuators for joints the robot doesn't have: {sorted(missing)}")
    default = _parse(path, "default", data["default"]) if "default" in data else None
    out = {}
    for j in joints:
        if j in per_joint:
            out[j] = _parse(path, f"joints.{j}", per_joint[j])
        elif default is None:
            raise ValueError(f"{path}: no actuator for joint {j!r} and no [default]")
        else:
            out[j] = default
    return out


def _parse(path: Path, where: str, table: dict) -> Actuator:
    table = dict(table)
    kind = table.pop("type", None)
    if kind not in TYPES:
        raise ValueError(f"{path} [{where}]: type must be one of {sorted(TYPES)}, got {kind!r}")
    try:
        return TYPES[kind](**table)
    except TypeError as e:  # unknown or missing field
        raise ValueError(f"{path} [{where}]: {e}") from None
