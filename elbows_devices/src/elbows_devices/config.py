"""Robot setup files: one TOML per physical rig, e.g. `hardware/setups/so101.toml` (not committed;
template `hardware/setups/example.toml`). Nothing here assumes a default setup, so the same code runs
any robot given its file.

One table per device on the rig, keyed by a name you choose. Its keys are whatever that device's
driver needs, e.g. a USB serial number for a servo bus, or a bus address and mounting for an IMU:

    [arm]
    usb_serial = "5C93150591"
"""

from __future__ import annotations

import tomllib
from pathlib import Path


def load_setup(path: str | Path) -> dict[str, dict]:
    """Device name -> its table."""
    path = Path(path).expanduser()
    if not path.is_file():
        raise FileNotFoundError(f"no setup file {path}; copy hardware/setups/example.toml and fill it in")
    with path.open("rb") as f:
        return tomllib.load(f)


def device(setup: str | Path, name: str) -> dict:
    devices = load_setup(setup)
    if name not in devices:
        raise KeyError(f"no [{name}] in {setup}; have {sorted(devices)}")
    return devices[name]
