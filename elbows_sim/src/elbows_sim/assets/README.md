# Shared sim assets

Robot and scene files reused across tasks. Robot builders that use them live in `../robots/`,
actuator kinds in `../core/actuators.py`, and file readers in `../formats/`. Task-specific assets stay in `../tasks/assets/`.

| Path | What | Source |
|---|---|---|
| `so101/` | Stock SO-101 follower URDF (`so101_new_calib.urdf`, joint zero = middle of range) and the 13 meshes it uses | [TheRobotStudio/SO-ARM100](https://github.com/TheRobotStudio/SO-ARM100) `Simulation/SO101` at `a758567`, Apache-2.0 (`so101/LICENSE`) |
| `so101/actuators.toml` | STS3215 servo on every joint: datasheet torque and speed plus stock SO-101 gains; its `source` key records where each came from | datasheet, TheRobotStudio, Open Duck Mini |
| `so101/hard_stops.toml` | Physical end stops (wrist roll), measured from the meshes with `scripts/hard_stop_sweep.py` | ours |
| `scenes/arm_tabletop.xml` | Floor, light, `front`/`top` cameras and a `robot_mount` site for URDF arms | ours |

Keep upstream files unmodified so they can be diffed against new releases.
