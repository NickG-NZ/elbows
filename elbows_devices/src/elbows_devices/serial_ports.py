"""Find serial devices (servo buses, USB IMUs, ...) by USB serial number, which, unlike the port path,
is the same on every OS, USB socket and replug: `/dev/cu.usbmodem…` on macOS, `/dev/ttyACM0`/`ACM1`
in plug order on Linux.
"""

from __future__ import annotations

from pathlib import Path

from serial.tools import list_ports

from .config import DEFAULT_CONFIG, device


def find_port(name: str, config: str | Path = DEFAULT_CONFIG) -> str:
    """Current port path of a device with a `usb_serial` in the config, e.g. find_port("so101_follower")."""
    serial = device(name, config).get("usb_serial")
    if not serial:
        raise KeyError(f"[{name}] in {config} has no usb_serial")
    ports = [p for p in list_ports.comports() if p.serial_number == serial]
    if not ports:
        connected = {p.device: p.serial_number for p in list_ports.comports() if p.serial_number}
        raise LookupError(f"{name} (USB serial {serial}) is not connected; connected: {connected}")
    # macOS can list a port as both /dev/cu.* and /dev/tty.*; cu.* doesn't wait for carrier detect.
    return min(p.device for p in ports)
