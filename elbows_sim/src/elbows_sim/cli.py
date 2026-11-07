"""`elbows-sim` command line: inspect tasks and backends. Training lives in `elbows-learn`."""

from __future__ import annotations

import argparse

from . import TASKS, available_backends
from .backends import BACKENDS


def main() -> None:
    p = argparse.ArgumentParser(prog="elbows-sim")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list", help="list tasks and backends")
    p.parse_args()
    print("tasks:   ", ", ".join(sorted(TASKS)))
    avail = set(available_backends())
    print("backends:", ", ".join(f"{b}{'' if b in avail else ' (unavailable)'}" for b in BACKENDS))


if __name__ == "__main__":
    main()
