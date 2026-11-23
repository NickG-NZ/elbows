"""Read actuator files (TOML, schema = `ActuatorModel`). See `assets/actuators/` for examples."""

from __future__ import annotations

import dataclasses
import tomllib
from pathlib import Path

from ..core.spec import ActuatorModel

CONTROL_MODES = ("position",)


def load_actuator(path: str | Path) -> ActuatorModel:
    path = Path(path)
    with path.open("rb") as f:
        data = tomllib.load(f)
    fields = {f.name: f for f in dataclasses.fields(ActuatorModel)}
    unknown = set(data) - set(fields)
    if unknown:
        raise ValueError(f"{path}: unknown keys {sorted(unknown)}; expected some of {sorted(fields)}")
    required = {n for n, f in fields.items() if f.default is dataclasses.MISSING}
    missing = required - set(data)
    if missing:
        raise ValueError(f"{path}: missing keys {sorted(missing)}")
    if data.get("control", "position") not in CONTROL_MODES:
        raise ValueError(f"{path}: control must be one of {CONTROL_MODES}, got {data['control']!r}")
    for n in ("kp", "kv", "stall_torque", "no_load_speed", "damping", "armature", "frictionloss"):
        if n in data:
            data[n] = float(data[n])
            if data[n] < 0:
                raise ValueError(f"{path}: {n} must be >= 0")
    for n in ("stall_torque", "no_load_speed"):
        if data[n] <= 0:
            raise ValueError(f"{path}: {n} must be > 0")
    return ActuatorModel(**data)
