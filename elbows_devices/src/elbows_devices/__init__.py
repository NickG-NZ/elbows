"""Drivers and discovery for physical robots and sensors: servo buses, IMUs, cameras, ...

Shared pieces live at the top level (`config` for robot setup files, `serial_ports`); each device or
device family gets its own module or subpackage as it is added (e.g. `imu/`).
"""

from .config import device, load_setup
from .serial_ports import find_port

__all__ = ["device", "find_port", "load_setup"]
