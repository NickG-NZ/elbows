from __future__ import annotations

from types import SimpleNamespace

import pytest

from elbows_devices import serial_ports as ports


@pytest.fixture
def config(tmp_path):
    f = tmp_path / "devices.toml"
    f.write_text('[so101_follower]\nusb_serial = "5C93150591"\n')
    return f


def _fake(monkeypatch, *devs):
    infos = [SimpleNamespace(device=d, serial_number=s) for d, s in devs]
    monkeypatch.setattr(ports.list_ports, "comports", lambda: infos)


def test_finds_port_by_serial_on_macos(monkeypatch, config):
    _fake(monkeypatch, ("/dev/tty.usbmodem5C931505911", "5C93150591"), ("/dev/cu.usbmodem5C931505911", "5C93150591"),
          ("/dev/cu.usbmodemOTHER1", "OTHER"))  # fmt: skip
    assert ports.find_port("so101_follower", config) == "/dev/cu.usbmodem5C931505911"


def test_finds_port_by_serial_on_linux(monkeypatch, config):
    _fake(monkeypatch, ("/dev/ttyACM0", "OTHER"), ("/dev/ttyACM1", "5C93150591"))
    assert ports.find_port("so101_follower", config) == "/dev/ttyACM1"


def test_errors(monkeypatch, config, tmp_path):
    _fake(monkeypatch, ("/dev/ttyACM0", "OTHER"))
    with pytest.raises(LookupError):
        ports.find_port("so101_follower", config)
    with pytest.raises(KeyError):
        ports.find_port("so101_leader", config)
    with pytest.raises(FileNotFoundError):
        ports.find_port("so101_follower", tmp_path / "missing.toml")
