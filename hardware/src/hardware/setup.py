"""Robot setups: one directory per physical rig, e.g. `hardware/setups/so101/` (local, not committed;
template `hardware/setups/example/`). Mirrors `rt-devices`' `setup` module.

    hardware/setups/<setup>/
      setup.toml                         devices on this rig, one table each
      calibration/<device>/<kind>.toml   this rig's in-situ calibration
    hardware/devices/<serial>/<kind>.toml  per-unit characterization, follows the hardware
"""

from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path

SETUP_FILE = "setup.toml"


@dataclass(frozen=True)
class Setup:
    dir: Path
    devices: dict[str, dict]

    @classmethod
    def load(cls, path: str | Path) -> Setup:
        """Load a setup from its directory (or its `setup.toml`)."""
        path = Path(path).expanduser()
        file = path / SETUP_FILE if path.is_dir() else path
        if not file.is_file():
            raise FileNotFoundError(f"no setup file {file}; copy hardware/setups/example/ and fill it in")
        with file.open("rb") as f:
            tables = tomllib.load(f)
        for name, value in tables.items():
            if not isinstance(value, dict):
                raise TypeError(f"{file}: top-level key {name!r} must be a [table], one per device")
        return cls(file.parent, tables)

    def device(self, name: str) -> dict:
        if name not in self.devices:
            raise KeyError(f"no [{name}] in {self.dir / SETUP_FILE}; have {sorted(self.devices)}")
        return self.devices[name]

    def calibration_path(self, device: str, kind: str) -> Path:
        """Where this rig's in-situ calibration of `kind` for `device` lives (may not exist yet)."""
        return self.dir / "calibration" / device / f"{kind}.toml"

    def characterization_path(self, serial: str, kind: str) -> Path:
        """Where the characterization of `kind` for the unit with this serial lives (may not exist yet)."""
        return self.dir.parent.parent / "devices" / serial / f"{kind}.toml"
