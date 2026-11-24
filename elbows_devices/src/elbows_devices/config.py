"""Local device config: `hardware/devices.toml` (not committed; copy `hardware/devices.example.toml`).

One table per physical device, keyed by a name you choose. Its keys are whatever that device's driver
needs to find and set it up, e.g. a USB serial number for a servo board, or a bus address and mounting
for an IMU:

    [so101_follower]
    usb_serial = "5C93150591"
"""

from __future__ import annotations

import tomllib
from pathlib import Path

DEFAULT_CONFIG = Path(__file__).parents[3] / "hardware" / "devices.toml"


def load_devices(path: str | Path = DEFAULT_CONFIG) -> dict[str, dict]:
    path = Path(path).expanduser()
    if not path.is_file():
        raise FileNotFoundError(f"{path} not found; copy devices.example.toml next to it and fill it in")
    with path.open("rb") as f:
        return tomllib.load(f)


def device(name: str, path: str | Path = DEFAULT_CONFIG) -> dict:
    devices = load_devices(path)
    if name not in devices:
        raise KeyError(f"no [{name}] in {path}; have {sorted(devices)}")
    return devices[name]
