"""Build an MjModel from a SceneSpec: plain MJCF, or an MJCF scene with a URDF robot attached.

Shared by the MuJoCo CPU and MJX backends.
"""

from __future__ import annotations

from pathlib import Path

import mujoco

from ..core.spec import RobotSpec, SceneSpec


def load_model(scene: SceneSpec) -> mujoco.MjModel:
    if "mjcf" not in scene.assets:
        raise ValueError(f"scene {scene.name!r} has no 'mjcf' asset; MuJoCo needs one")
    if scene.robot is None:
        m = mujoco.MjModel.from_xml_path(str(scene.assets["mjcf"]))
    else:
        spec = mujoco.MjSpec.from_file(str(scene.assets["mjcf"]))
        _attach_robot(spec, scene.robot)
        m = spec.compile()
    m.opt.timestep = scene.physics_dt
    return m


def _attach_robot(spec: mujoco.MjSpec, robot: RobotSpec) -> None:
    urdf = mujoco.MjSpec.from_file(str(robot.urdf))
    urdf.compiler.fusestatic = False  # keep fixed links (e.g. gripper_frame_link) as named frames
    for mesh in urdf.meshes:
        mesh.file = _resolve_mesh(mesh.file, robot.urdf)
    if not robot.self_collision:
        # contype=1, conaffinity=0: robot geoms skip each other but still hit scene geoms (1/1 by default).
        for g in urdf.geoms:
            if g.contype or g.conaffinity:
                g.contype, g.conaffinity = 1, 0

    site = spec.site(robot.mount)
    if site is None:
        raise KeyError(f"scene has no site {robot.mount!r} to mount the robot on")
    spec.attach(urdf, prefix="", site=site)

    joints = {j.name: j for j in spec.joints}
    for name, servo in robot.actuators.items():
        j = joints.get(name)
        if j is None:
            raise KeyError(f"joint {name!r} not found in {robot.urdf.name}")
        j.damping = [servo.damping, 0.0, 0.0]
        j.armature = servo.armature
        j.frictionloss = servo.frictionloss
        a = spec.add_actuator(name=name, target=name, trntype=mujoco.mjtTrn.mjTRN_JOINT)
        a.set_to_position(kp=servo.kp, kv=servo.kv)
        a.forcelimited, a.forcerange = True, [-servo.force_limit, servo.force_limit]
        if not (j.range[0] < j.range[1]):
            raise ValueError(f"joint {name!r} needs finite limits in the URDF so actions can map onto them")
        a.ctrllimited, a.ctrlrange = True, j.range


def _resolve_mesh(file: str, urdf: Path) -> str:
    """Make mesh paths absolute. `package://<pkg>/x.stl` resolves to `<urdf dir>/x.stl`, i.e. the
    URDF is assumed to sit at the package root, which is what CAD exporters usually produce."""
    if file.startswith("package://"):
        file = file.removeprefix("package://").partition("/")[2]
    elif file.startswith("file://"):
        file = file.removeprefix("file://")
    p = Path(file)
    return str(p if p.is_absolute() else (urdf.parent / p).resolve())
