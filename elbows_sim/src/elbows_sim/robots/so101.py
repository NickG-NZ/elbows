"""SO-101 arm: the stock model by default, or any URDF of a modified design.

    so101_robot()                          # stock 6-joint follower (assets/so101/)
    so101_robot("path/to/so101_6dof.urdf") # e.g. with a forearm roll; joints are read from the file

Every actuated joint gets `default_actuator` (STS3215) unless `actuators` overrides it by joint name.
Actuators are library names, paths to TOML files, or ActuatorModels:

    so101_robot(urdf, actuators={"elbow_rotate": "sts3215", "gripper": "~/servos/sts3215_gripper.toml"})
"""

from __future__ import annotations

from pathlib import Path

from ..core.spec import ActuatorModel, RobotSpec
from ..formats.urdf import urdf_joints
from .actuators import actuator

ASSETS = Path(__file__).parents[1] / "assets"
STOCK_URDF = ASSETS / "so101" / "so101_new_calib.urdf"
SCENE = ASSETS / "scenes" / "arm_tabletop.xml"


ActuatorRef = str | Path | ActuatorModel


def so101_robot(
    urdf: str | Path | None = None,
    actuators: dict[str, ActuatorRef] | None = None,
    default_actuator: ActuatorRef = "sts3215",
    self_collision: bool = False,
) -> RobotSpec:
    path = Path(urdf).expanduser().resolve() if urdf else STOCK_URDF
    if not path.is_file():
        raise FileNotFoundError(path)
    joints = [j.name for j in urdf_joints(path)]
    overrides = actuators or {}
    unknown = set(overrides) - set(joints)
    if unknown:
        raise KeyError(f"actuators given for joints not in {path.name}: {sorted(unknown)}")
    default = actuator(default_actuator)
    resolved = {j: actuator(overrides[j]) if j in overrides else default for j in joints}
    return RobotSpec(urdf=path, actuators=resolved, self_collision=self_collision)
