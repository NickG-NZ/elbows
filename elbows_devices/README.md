# elbows_devices

Drivers and discovery for physical hardware: robot arms, IMUs, cameras and other sensors. Code only.
Each robot setup has its own config file (serial numbers, addresses, calibration),
`hardware/setups/<setup>.toml`, not committed. Nothing assumes a default: tools take `--setup FILE`. Setup notes live in [`hardware/`](../hardware/README.md).

- `config.py` reads a setup file: one table per device, holding whatever its driver needs.
- `serial_ports.py` finds a serial device's current port from its USB serial number, on any OS.
- New devices get their own module or subpackage (e.g. `imu/`).
  Keep heavy or vendor SDKs in optional extras.

```bash
uv run elbows-devices list                   # connected serial ports and their USB serials
uv run elbows-devices port --setup hardware/setups/so101.toml arm   # current port of a device
```
