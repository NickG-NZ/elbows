"""Find serial devices (servo buses, USB IMUs, ...) by USB serial number, which, unlike the port path,
is the same on every OS, USB socket and replug: `/dev/cu.usbmodem…` on macOS, `/dev/ttyACM0`/`ACM1`
in plug order on Linux.
"""

from __future__ import annotations

from serial.tools import list_ports


def find_port(usb_serial: str) -> str:
    """Current port path of the connected device with this USB serial number."""
    ports = [p for p in list_ports.comports() if p.serial_number == usb_serial]
    if not ports:
        connected = {p.device: p.serial_number for p in list_ports.comports() if p.serial_number}
        raise LookupError(f"no device with USB serial {usb_serial} connected; connected: {connected}")
    # macOS can list a port as both /dev/cu.* and /dev/tty.*; cu.* doesn't wait for carrier detect.
    return min(p.device for p in ports)
