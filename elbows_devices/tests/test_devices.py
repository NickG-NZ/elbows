from __future__ import annotations

from types import SimpleNamespace

import pytest

from elbows_devices import device, find_port, load_setup, serial_ports


def _connected(monkeypatch, *devs):
    infos = [SimpleNamespace(device=d, serial_number=s) for d, s in devs]
    monkeypatch.setattr(serial_ports.list_ports, "comports", lambda: infos)


def test_finds_port_by_serial_on_macos(monkeypatch):
    _connected(monkeypatch, ("/dev/tty.usbmodem5C931505911", "5C93150591"),
               ("/dev/cu.usbmodem5C931505911", "5C93150591"), ("/dev/cu.usbmodemX1", "X"))  # fmt: skip
    assert find_port("5C93150591") == "/dev/cu.usbmodem5C931505911"


def test_finds_port_by_serial_on_linux(monkeypatch):
    _connected(monkeypatch, ("/dev/ttyACM0", "X"), ("/dev/ttyACM1", "5C93150591"))
    assert find_port("5C93150591") == "/dev/ttyACM1"
    with pytest.raises(LookupError):
        find_port("NOPE")


def test_each_setup_is_its_own_file(tmp_path):
    a, b = tmp_path / "desk.toml", tmp_path / "lab.toml"
    a.write_text('[arm]\nusb_serial = "A"\n')
    b.write_text('[arm]\nusb_serial = "B"\n[imu]\nbus = 1\naddress = 0x68\n')
    assert device(a, "arm")["usb_serial"] == "A" and device(b, "arm")["usb_serial"] == "B"
    assert load_setup(b)["imu"] == {"bus": 1, "address": 0x68}
    with pytest.raises(KeyError):
        device(a, "imu")
    with pytest.raises(FileNotFoundError):
        load_setup(tmp_path / "missing.toml")
