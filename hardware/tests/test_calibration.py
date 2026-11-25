from __future__ import annotations

from pathlib import Path

import pytest

from hardware import Calibration, CalibrationError, Setup

# Shared with the Rust tests (rt-devices), so both readers stay in sync.
EXAMPLE = Path(__file__).parent / "fixtures" / "example_calibration.toml"


def test_reads_the_shared_example():
    cal = Calibration.load(EXAMPLE, "example", 1)
    assert (cal.device, cal.serial, cal.quality) == ("arm", "5C93150591", {"residual_rms": 0.5})
    assert cal.data["joints"][0] == "shoulder_pan"
    assert len(cal.data["joints"]) == len(cal.data["offset"])


def test_save_round_trips(tmp_path):
    cal = Calibration.load(EXAMPLE, "example", 1)
    path = cal.save(tmp_path / "a" / "b" / "example.toml")
    assert Calibration.load(path, "example", 1) == cal
    bare = Calibration(kind="example", version=1, device="arm", data={"x": 1.0})
    assert Calibration.load(bare.save(tmp_path / "bare.toml"), "example", 1) == bare
    assert not list(tmp_path.rglob("*.tmp"))


def test_refuses_wrong_kind_version_or_bad_header(tmp_path):
    with pytest.raises(CalibrationError, match="expected 'joint_ranges'"):
        Calibration.load(EXAMPLE, "joint_ranges", 1)
    with pytest.raises(CalibrationError, match="recalibrate"):
        Calibration.load(EXAMPLE, "example", 2)
    with pytest.raises(FileNotFoundError):
        Calibration.load(tmp_path / "missing.toml", "example", 1)
    head = '[calibration]\nkind = "example"\ndevice = "a"\ncreated = 2026-01-01T00:00:00Z\n'
    for text, err in [
        ('[calibration]\nkind = "example"\nversion = 1\n[data]\n', "missing 'device'"),
        (head + 'version = "1"\n[data]\n', "'version' must be int"),
        (head + "version = 1\nextra = 1\n[data]\n", "unknown"),
        (head + "version = 1\n", "exactly"),
    ]:
        bad = tmp_path / "bad.toml"
        bad.write_text(text)
        with pytest.raises(CalibrationError, match=err):
            Calibration.load(bad, "example", 1)


def test_setup_paths(tmp_path):
    setup_dir = tmp_path / "hardware" / "setups" / "desk"
    setup_dir.mkdir(parents=True)
    (setup_dir / "setup.toml").write_text('[arm]\nusb_serial = "A"\n[imu]\nbus = 1\n')
    setup = Setup.load(setup_dir)
    assert Setup.load(setup_dir / "setup.toml") == setup
    assert setup.device("arm")["usb_serial"] == "A"
    assert setup.calibration_path("arm", "joint_ranges") == setup_dir / "calibration/arm/joint_ranges.toml"
    assert setup.characterization_path("A", "motor") == tmp_path / "hardware/devices/A/motor.toml"
    with pytest.raises(KeyError):
        setup.device("camera")
    with pytest.raises(FileNotFoundError):
        Setup.load(tmp_path / "nope")
