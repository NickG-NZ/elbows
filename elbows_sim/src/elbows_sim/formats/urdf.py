"""Read what tasks need from a URDF (joint names, order, limits) with the standard library only.

Simulator-specific loading lives in `backends/`. This module only parses, so tasks can size their
action and observation spaces without importing a simulator.
"""

from __future__ import annotations

import math
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path

ACTUATED_TYPES = ("revolute", "continuous", "prismatic")


@dataclass(frozen=True)
class UrdfJoint:
    name: str
    type: str
    parent: str
    child: str
    lower: float
    upper: float


def urdf_joints(path: str | Path) -> list[UrdfJoint]:
    """Actuated joints in kinematic-tree order (depth-first from the root link, siblings in file order).

    File order is unreliable (exporters often write leaves first). Tree order gives, for the SO-101,
    shoulder_pan, shoulder_lift, elbow_flex, wrist_flex, wrist_roll, gripper, and puts an added
    joint (e.g. a forearm roll) where it sits in the chain.
    """
    root = ET.parse(path).getroot()
    children: dict[str, list[ET.Element]] = {}
    for j in root.findall("joint"):
        children.setdefault(j.find("parent").get("link"), []).append(j)
    child_links = {j.find("child").get("link") for js in children.values() for j in js}
    roots = [link.get("name") for link in root.findall("link") if link.get("name") not in child_links]
    if len(roots) != 1:
        raise ValueError(f"{path}: expected one root link, found {roots}")

    out: list[UrdfJoint] = []

    def visit(link: str) -> None:
        for j in children.get(link, []):
            child = j.find("child").get("link")
            if j.get("type") in ACTUATED_TYPES:
                lim = j.find("limit")
                bounded = lim is not None and j.get("type") != "continuous"
                lo = float(lim.get("lower", -math.inf)) if bounded else -math.inf
                hi = float(lim.get("upper", math.inf)) if bounded else math.inf
                out.append(UrdfJoint(j.get("name"), j.get("type"), link, child, lo, hi))
            visit(child)

    visit(roots[0])
    return out
