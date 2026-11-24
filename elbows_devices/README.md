# elbows_devices

Drivers and discovery for physical hardware: robot arms, IMUs, cameras and other sensors. Code only.
Per-machine config (serial numbers, addresses, calibration) stays in `hardware/devices.toml`, which
is not committed. Setup notes live in [`hardware/`](../hardware/README.md).

- `config.py` reads `devices.toml`: one table per device, holding whatever its driver needs.
- `serial_ports.py` finds a serial device's current port from its USB serial number, on any OS.
- New devices get their own module or subpackage (e.g. `imu/`).
  Keep heavy or vendor SDKs in optional extras.

```bash
uv run elbows-devices list                   # connected serial ports and their USB serials
uv run elbows-devices port so101_follower    # current port of a configured device
```
