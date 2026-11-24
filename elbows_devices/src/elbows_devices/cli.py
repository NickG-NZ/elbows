"""elbows-devices list                          connected serial ports and their USB serial numbers
elbows-devices port --setup FILE <device>    current port of a device in a robot setup file

    uv run lerobot-calibrate --robot.type=so101_follower --robot.id=follower \\
      --robot.port=$(uv run elbows-devices port --setup hardware/setups/so101.toml arm)
"""

from __future__ import annotations

import argparse
import sys

from serial.tools import list_ports

from .config import device
from .serial_ports import find_port


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    port = sub.add_parser("port")
    port.add_argument("--setup", required=True, help="robot setup file, e.g. hardware/setups/so101.toml")
    port.add_argument("device", help="a table name in the setup file")
    a = p.parse_args()

    if a.cmd == "list":
        for info in sorted(list_ports.comports(), key=lambda i: i.device):
            print(f"{info.device}\tserial={info.serial_number or '-'}\t{info.description}")
        return
    try:
        serial = device(a.setup, a.device).get("usb_serial")
        if not serial:
            sys.exit(f"[{a.device}] in {a.setup} has no usb_serial")
        print(find_port(serial))
    except (FileNotFoundError, KeyError, LookupError) as e:
        sys.exit(str(e))
