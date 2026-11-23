"""Read joint hard-stop files: TOML tables of physical end stops, in degrees.

[wrist_roll]
lower = -163.4
upper = 170.4
source = "how it was measured"
"""

from __future__ import annotations

import math
import tomllib
from pathlib import Path


def load_hard_stops(path: str | Path) -> dict[str, tuple[float, float]]:
    """Joint name -> (lower, upper) in radians."""
    path = Path(path)
    with path.open("rb") as f:
        data = tomllib.load(f)
    out = {}
    for joint, entry in data.items():
        unknown = set(entry) - {"lower", "upper", "source"}
        if unknown or not {"lower", "upper"} <= set(entry):
            raise ValueError(f"{path} [{joint}]: need lower and upper (degrees), optional source; got {sorted(entry)}")
        lo, hi = float(entry["lower"]), float(entry["upper"])
        if not lo < hi:
            raise ValueError(f"{path} [{joint}]: lower must be < upper")
        out[joint] = (math.radians(lo), math.radians(hi))
    return out
