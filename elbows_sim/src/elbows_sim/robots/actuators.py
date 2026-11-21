"""The actuator library: servo files in `assets/actuators/<name>.toml`, looked up by name.

Add a servo by adding a TOML file there. Robots refer to it by name or by path to any TOML file.
"""

from __future__ import annotations

from pathlib import Path

from ..core.spec import ActuatorModel
from ..formats.actuator import load_actuator

LIBRARY = Path(__file__).parents[1] / "assets" / "actuators"


def actuator(ref: str | Path | ActuatorModel) -> ActuatorModel:
    """Resolve a library name ("sts3215"), a path to a TOML file, or an ActuatorModel as-is."""
    if isinstance(ref, ActuatorModel):
        return ref
    path = Path(ref).expanduser()
    if path.suffix != ".toml":
        path = LIBRARY / f"{ref}.toml"
        if not path.is_file():
            raise KeyError(f"no actuator {ref!r} in {LIBRARY}; have {sorted(available_actuators())}")
    return load_actuator(path)


def available_actuators() -> list[str]:
    return sorted(p.stem for p in LIBRARY.glob("*.toml"))


STS3215 = actuator("sts3215")
