"""Calibration files: the contract between calibration procedures (here) and the drivers that use the
results (`rt-devices`' `calibration` module, which reads the same format). Calibration data is local,
never committed.

- **in-situ**, tied to one rig (joint offsets and ranges, camera mounting pose):
  `Setup.calibration_path(device, kind)`
- **characterization**, tied to one physical unit (camera intrinsics, motor parameters):
  `Setup.characterization_path(serial, kind)`

Every file is a `[calibration]` header plus a `[data]` table whose layout the `kind` defines. A reader
states the kind and the `[data]` layout version it understands; anything else is refused.

    [calibration]
    kind = "joint_ranges"
    version = 1
    device = "arm"
    serial = "5C93150591"           # optional
    created = 2026-11-25T10:00:00Z
    method = "manual range sweep"   # optional
    [calibration.quality]           # optional
    [data]
"""

from __future__ import annotations

import os
import tomllib
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import tomli_w

_REQUIRED = {"kind": str, "version": int, "device": str, "created": datetime}
_OPTIONAL = {"serial": str, "method": str, "quality": dict}


class CalibrationError(ValueError):
    pass


def _now() -> datetime:
    return datetime.now(UTC).replace(microsecond=0)


@dataclass
class Calibration:
    kind: str
    version: int
    device: str
    data: dict[str, Any]
    serial: str | None = None
    method: str = ""
    quality: dict[str, float] = field(default_factory=dict)
    created: datetime = field(default_factory=_now)

    @classmethod
    def load(cls, path: str | Path, kind: str, version: int) -> Calibration:
        """Load a calibration of `kind`, refusing any `[data]` layout version but `version`."""
        path = Path(path)
        if not path.is_file():
            raise FileNotFoundError(f"no calibration file {path}; run its calibration procedure")
        with path.open("rb") as f:
            doc = tomllib.load(f)
        if set(doc) != {"calibration", "data"}:
            raise CalibrationError(f"{path}: expected exactly [calibration] and [data], got {sorted(doc)}")
        header = doc["calibration"]
        unknown = set(header) - set(_REQUIRED) - set(_OPTIONAL)
        if unknown:
            raise CalibrationError(f"{path}: unknown [calibration] keys {sorted(unknown)}")
        for key, typ in {**_REQUIRED, **_OPTIONAL}.items():
            if key in _REQUIRED and key not in header:
                raise CalibrationError(f"{path}: [calibration] is missing {key!r}")
            if key in header and not isinstance(header[key], typ):
                raise CalibrationError(f"{path}: [calibration] {key!r} must be {typ.__name__}")
        if header["kind"] != kind:
            raise CalibrationError(f"{path}: holds {header['kind']!r} calibration, expected {kind!r}")
        if header["version"] != version:
            raise CalibrationError(
                f"{path}: {kind} version {header['version']}, this code understands version {version}; recalibrate"
            )
        return cls(data=doc["data"], **header)

    def save(self, path: str | Path) -> Path:
        """Write atomically (a crash mid-write never leaves a half-written calibration)."""
        path = Path(path)
        header: dict[str, Any] = {
            "kind": self.kind,
            "version": self.version,
            "device": self.device,
            "created": self.created,
        }
        if self.serial is not None:
            header["serial"] = self.serial
        if self.method:
            header["method"] = self.method
        if self.quality:
            header["quality"] = self.quality
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(path.suffix + ".tmp")
        tmp.write_bytes(tomli_w.dumps({"calibration": header, "data": self.data}).encode())
        os.replace(tmp, path)
        return path
