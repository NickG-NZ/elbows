# SO-101

Everything that describes the SO-101 arm (any unit, not one in particular). Per-unit values, such as
system-identified friction or calibration, belong in `hardware/` calibration: per unit under
`hardware/devices/<serial>/`, per rig under `hardware/setups/<setup>/calibration/` (see `hardware/README.md`).

| Path | What | Source |
|---|---|---|
| `upstream/` | Stock follower URDF (`so101_new_calib.urdf`, joint zero = middle of range), the 13 meshes it uses, LICENSE. Unmodified, so it can be diffed against new releases | [TheRobotStudio/SO-ARM100](https://github.com/TheRobotStudio/SO-ARM100) `Simulation/SO101` at `a758567`, Apache-2.0 |
| `actuators.toml` | STS3215 servo on every joint: datasheet torque and speed plus stock gains; its `source` key records where each came from | datasheet, TheRobotStudio, Open Duck Mini |
| `hard_stops.toml` | Physical end stops (wrist roll), measured from the meshes with `elbows_sim/scripts/hard_stop_sweep.py` | ours |
| `__init__.py` | `so101_robot()`: builds a `RobotSpec` from the stock URDF or a custom one, plus the files above | ours |
