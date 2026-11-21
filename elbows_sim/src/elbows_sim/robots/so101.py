"""SO-101 arm: the stock model by default, or any URDF of a modified design.

    so101_robot()                          # stock 6-joint follower (assets/so101/)
    so101_robot("path/to/so101_6dof.urdf") # e.g. with a forearm roll; joints are read from the file

Every actuated joint gets an STS3215 unless `actuators` overrides it by joint name.
"""

from __future__ import annotations

from pathlib import Path

from ..core.spec import ActuatorModel, RobotSpec
from ..formats.urdf import urdf_joints
from .actuators import STS3215

ASSETS = Path(__file__).parents[1] / "assets"
STOCK_URDF = ASSETS / "so101" / "so101_new_calib.urdf"
SCENE = ASSETS / "scenes" / "arm_tabletop.xml"


def so101_robot(
    urdf: str | Path | None = None, actuators: dict[str, ActuatorModel] | None = None, self_collision: bool = False
) -> RobotSpec:
    path = Path(urdf).expanduser().resolve() if urdf else STOCK_URDF
    if not path.is_file():
        raise FileNotFoundError(path)
    joints = [j.name for j in urdf_joints(path)]
    overrides = actuators or {}
    unknown = set(overrides) - set(joints)
    if unknown:
        raise KeyError(f"actuators given for joints not in {path.name}: {sorted(unknown)}")
    return RobotSpec(urdf=path, actuators={j: overrides.get(j, STS3215) for j in joints}, self_collision=self_collision)
