"""Shared surroundings (floor, lights, cameras, mount sites) that robots are placed into."""

from pathlib import Path

HERE = Path(__file__).parent
ARM_TABLETOP = HERE / "arm_tabletop.xml"  # floor, light, front/top cameras, `robot_mount` site for URDF arms

__all__ = ["ARM_TABLETOP"]
