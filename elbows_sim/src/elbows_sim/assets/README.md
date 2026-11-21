# Shared sim assets

Robot and scene files reused across tasks. Python descriptions that use them (servo models, robot
builders) live in `../robots/`, and file readers (URDF) in `../formats/`. Task-specific assets stay in `../tasks/assets/`.

| Path | What | Source |
|---|---|---|
| `so101/` | Stock SO-101 follower URDF (`so101_new_calib.urdf`, joint zero = middle of range) and the 13 meshes it uses | [TheRobotStudio/SO-ARM100](https://github.com/TheRobotStudio/SO-ARM100) `Simulation/SO101` at `a758567`, Apache-2.0 (`so101/LICENSE`) |
| `actuators/*.toml` | Servo models (schema: `ActuatorModel`), looked up by file name. `sts3215.toml` has the stock SO-101 values; its `source` key records where they came from | TheRobotStudio/Open Duck Mini |
| `scenes/arm_tabletop.xml` | Floor, light, `front`/`top` cameras and a `robot_mount` site for URDF arms | ours |

Keep upstream files unmodified so they can be diffed against new releases.
