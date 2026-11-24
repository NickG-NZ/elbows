"""elbows-devices list            connected serial ports and their USB serial numbers
elbows-devices port <device>   current port of a device in hardware/devices.toml

    uv run lerobot-calibrate --robot.type=so101_follower \\
      --robot.port=$(uv run elbows-devices port so101_follower) --robot.id=follower
"""

from __future__ import annotations

import argparse
import sys

from serial.tools import list_ports

from .serial_ports import find_port


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    port = sub.add_parser("port")
    port.add_argument("device")
    port.add_argument("--config", default=None)
    a = p.parse_args()

    if a.cmd == "list":
        for info in sorted(list_ports.comports(), key=lambda i: i.device):
            print(f"{info.device}\tserial={info.serial_number or '-'}\t{info.description}")
        return
    try:
        print(find_port(a.device, a.config) if a.config else find_port(a.device))
    except (FileNotFoundError, KeyError, LookupError) as e:
        sys.exit(str(e))
