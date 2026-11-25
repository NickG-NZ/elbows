"""Calibration procedures and robot setups for physical hardware.

Drivers live in Rust (`elbows_rt`, `rt-devices`). This package holds what runs outside the real-time
path: calibration procedures and the setup and calibration file formats they share with the drivers.
"""

from .calibration import Calibration, CalibrationError
from .setup import Setup

__all__ = ["Calibration", "CalibrationError", "Setup"]
