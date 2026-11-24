"""SO-101 arm: the stock model by default, or any URDF of a modified design.

    so101_robot()                          # stock 6-joint follower (upstream/)
    so101_robot("path/to/so101_6dof.urdf") # e.g. with a forearm roll; joints are read from the file

What the URDF doesn't say lives in TOML files in this folder: `actuators.toml` (STS3215 servos on every
joint) and `hard_stops.toml`. A modified arm uses the stock files, or its own:

    so101_robot(urdf, actuators="~/Documents/so101-6dof/actuators.toml")
"""

from __future__ import annotations

from pathlib import Path

from ...core.spec import RobotSpec
from ...formats.actuators import load_actuators
from ...formats.limits import load_hard_stops
from ...formats.urdf import urdf_joints

HERE = Path(__file__).parent
STOCK_URDF = HERE / "upstream" / "so101_new_calib.urdf"
STOCK_ACTUATORS = HERE / "actuators.toml"
STOCK_HARD_STOPS = HERE / "hard_stops.toml"


def so101_robot(
    urdf: str | Path | None = None,
    actuators: str | Path | None = None,
    hard_stops: str | Path | None = STOCK_HARD_STOPS,
    self_collision: bool = True,
) -> RobotSpec:
    """`actuators`: the robot's actuator file (default: the stock STS3215s). The stock hard stops are
    applied to the joints the URDF has, so they also fit modified arms that keep the stock wrist."""
    path = Path(urdf).expanduser().resolve() if urdf else STOCK_URDF
    if not path.is_file():
        raise FileNotFoundError(path)
    joints = [j.name for j in urdf_joints(path)]
    stops = load_hard_stops(Path(hard_stops).expanduser()) if hard_stops else {}
    return RobotSpec(
        urdf=path,
        actuators=load_actuators(Path(actuators or STOCK_ACTUATORS).expanduser(), joints),
        hard_stops={j: s for j, s in stops.items() if j in joints},
        self_collision=self_collision,
    )
