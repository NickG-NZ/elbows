"""Build an MjModel from a SceneSpec: plain MJCF, or an MJCF scene with a URDF robot attached.

Shared by the MuJoCo CPU and MJX backends.
"""

from __future__ import annotations

import warnings
from pathlib import Path

import mujoco

from ..core.spec import RobotSpec, SceneSpec
from .mujoco_actuators import add_actuator

# Collision uses each mesh's convex hull. CAD meshes give hulls of thousands of vertices, which
# makes MJX's mesh-mesh collision run out of memory and slows the CPU backend; cap them (both
# backends, so they agree). Rendering still uses the full meshes.
HULL_VERTS = 64


def load_model(scene: SceneSpec) -> mujoco.MjModel:
    if "mjcf" not in scene.assets:
        raise ValueError(f"scene {scene.name!r} has no 'mjcf' asset; MuJoCo needs one")
    if scene.robot is None:
        m = mujoco.MjModel.from_xml_path(str(scene.assets["mjcf"]))
    else:
        spec = mujoco.MjSpec.from_file(str(scene.assets["mjcf"]))
        _attach_robot(spec, scene.robot)
        with warnings.catch_warnings():  # same empty-prefix warning as in _attach_robot
            warnings.filterwarnings("ignore", message=".*empty prefix")
            m = spec.compile()
    m.opt.timestep = scene.physics_dt
    return m


def _attach_robot(spec: mujoco.MjSpec, robot: RobotSpec) -> None:
    urdf = mujoco.MjSpec.from_file(str(robot.urdf))
    urdf.compiler.fusestatic = False  # keep fixed links (e.g. gripper_frame_link) as named frames
    for mesh in urdf.meshes:
        mesh.file = _resolve_mesh(mesh.file, robot.urdf)
        mesh.maxhullvert = HULL_VERTS
    if not robot.self_collision:
        # contype=1, conaffinity=0: robot geoms skip each other but still hit scene geoms (1/1 by default).
        for g in urdf.geoms:
            if g.contype or g.conaffinity:
                g.contype, g.conaffinity = 1, 0

    site = spec.site(robot.mount)
    if site is None:
        raise KeyError(f"scene has no site {robot.mount!r} to mount the robot on")
    # An empty prefix keeps the URDF's names, which tasks rely on. MuJoCo warns that such a model may
    # not save back to XML (we never save it) and writes the warning to MUJOCO_LOG.TXT in the cwd.
    previous = mujoco.get_mju_user_warning()
    mujoco.set_mju_user_warning(lambda msg: None if "empty prefix" in msg else print(f"MuJoCo warning: {msg}"))
    try:
        spec.attach(urdf, prefix="", site=site)
    finally:
        mujoco.set_mju_user_warning(previous)

    if robot.self_collision:
        # MuJoCo skips parent-child contacts, except when the parent is welded to the world (the base),
        # so exclude every adjacent pair explicitly: their meshes overlap at the joint by design.
        for b in urdf.bodies:
            if b.name != "world" and b.parent is not None and b.parent.name != "world":
                spec.add_exclude(bodyname1=b.parent.name, bodyname2=b.name)

    joints = {j.name: j for j in spec.joints}
    for name, servo in robot.actuators.items():
        if name not in joints:
            raise KeyError(f"joint {name!r} not found in {robot.urdf.name}")
        add_actuator(spec, joints[name], servo, robot.hard_stops.get(name))


def _resolve_mesh(file: str, urdf: Path) -> str:
    """Make mesh paths absolute. `package://<pkg>/x.stl` resolves to `<urdf dir>/x.stl`, i.e. the
    URDF is assumed to sit at the package root, which is what CAD exporters usually produce."""
    if file.startswith("package://"):
        file = file.removeprefix("package://").partition("/")[2]
    elif file.startswith("file://"):
        file = file.removeprefix("file://")
    p = Path(file)
    return str(p if p.is_absolute() else (urdf.parent / p).resolve())
