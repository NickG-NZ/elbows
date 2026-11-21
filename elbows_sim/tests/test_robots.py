"""URDF robots: the stock SO-101 and arbitrary custom URDFs (e.g. an added forearm roll) load and drive."""

from __future__ import annotations

import struct
from pathlib import Path

import pytest
import torch
from elbows_sim.core.spec import ActuatorModel
from elbows_sim.formats import urdf_joints
from elbows_sim.robots import SO101_STOCK_URDF, STS3215, so101_robot

import elbows_sim

STOCK_JOINTS = ("shoulder_pan", "shoulder_lift", "elbow_flex", "wrist_flex", "wrist_roll", "gripper")
BACKENDS = [b for b in elbows_sim.available_backends() if b in ("mujoco", "mjx")]

_CUBE_TRIS = [
    ((0, 0, 0), (1, 0, 0), (1, 1, 0)), ((0, 0, 0), (1, 1, 0), (0, 1, 0)), ((0, 0, 1), (1, 1, 1), (1, 0, 1)),
    ((0, 0, 1), (0, 1, 1), (1, 1, 1)), ((0, 0, 0), (1, 0, 1), (1, 0, 0)), ((0, 0, 0), (0, 0, 1), (1, 0, 1)),
    ((0, 1, 0), (1, 1, 0), (1, 1, 1)), ((0, 1, 0), (1, 1, 1), (0, 1, 1)), ((0, 0, 0), (0, 1, 0), (0, 1, 1)),
    ((0, 0, 0), (0, 1, 1), (0, 0, 1)), ((1, 0, 0), (1, 0, 1), (1, 1, 1)), ((1, 0, 0), (1, 1, 1), (1, 1, 0)),
]  # fmt: skip


def _binary_stl(tris) -> bytes:  # MuJoCo reads binary STL only
    body = b"".join(struct.pack("<12fH", 0, 0, 0, *a, *b, *c, 0) for a, b, c in tris)
    return b"\0" * 80 + struct.pack("<I", len(tris)) + body


def _link(name: str, mesh: str | None = None) -> str:
    shape = f'<mesh filename="{mesh}" scale="0.02 0.02 0.02"/>' if mesh else '<box size="0.02 0.02 0.05"/>'
    return f"""<link name="{name}">
      <inertial><origin xyz="0 0 0.025"/><mass value="0.05"/><inertia ixx="1e-5" iyy="1e-5" izz="1e-5" ixy="0" ixz="0" iyz="0"/></inertial>
      <visual><geometry>{shape}</geometry></visual><collision><geometry>{shape}</geometry></collision>
    </link>"""


def _joint(name: str, parent: str, child: str, axis: str, kind: str = "revolute") -> str:
    return f"""<joint name="{name}" type="{kind}"><parent link="{parent}"/><child link="{child}"/>
      <origin xyz="0 0 0.05"/><axis xyz="{axis}"/><limit lower="-1.5" upper="1.5" effort="3" velocity="5"/></joint>"""


@pytest.fixture
def six_dof_urdf(tmp_path: Path) -> Path:
    """A stand-in for a modified SO-101: stock joint names plus `elbow_rotate` (forearm roll), written
    leaf-first like many exporters, with one mesh referenced through package://."""
    (tmp_path / "meshes").mkdir()
    (tmp_path / "meshes" / "cube.stl").write_bytes(_binary_stl(_CUBE_TRIS))
    chain = [
        ("shoulder_pan", "0 0 1"), ("shoulder_lift", "0 1 0"), ("elbow_flex", "0 1 0"),
        ("elbow_rotate", "0 0 1"), ("wrist_flex", "0 1 0"), ("wrist_roll", "0 0 1"), ("gripper", "1 0 0"),
    ]  # fmt: skip
    links = ["base_link"] + [f"link_{n}" for n, _ in chain]
    parts = [_link(lk, "package://so101_6dof/meshes/cube.stl" if i == 3 else None) for i, lk in enumerate(links)]
    parts += [_joint(n, links[i], links[i + 1], ax) for i, (n, ax) in enumerate(chain)]
    parts.append(_link("tool_frame") + _joint("tool_frame_joint", links[-1], "tool_frame", "1 0 0", "fixed"))
    path = tmp_path / "so101_6dof.urdf"
    path.write_text('<robot name="so101_6dof">' + "".join(reversed(parts)) + "</robot>")
    return path


def test_stock_urdf_joint_order():
    assert tuple(j.name for j in urdf_joints(SO101_STOCK_URDF)) == STOCK_JOINTS


def test_custom_urdf_joint_order_follows_the_chain(six_dof_urdf):
    names = [j.name for j in urdf_joints(six_dof_urdf)]
    assert names == [
        "shoulder_pan",
        "shoulder_lift",
        "elbow_flex",
        "elbow_rotate",
        "wrist_flex",
        "wrist_roll",
        "gripper",
    ]


def test_actuator_overrides():
    strong = ActuatorModel("strong", kp=50, kv=1, force_limit=10)
    r = so101_robot(actuators={"gripper": strong})
    assert r.actuators["gripper"] is strong and r.actuators["shoulder_pan"] is STS3215
    with pytest.raises(KeyError):
        so101_robot(actuators={"not_a_joint": strong})


@pytest.mark.parametrize("backend", BACKENDS)
def test_custom_urdf_loads_and_tracks(backend, six_dof_urdf):
    env = elbows_sim.make("so101_pose", backend=backend, num_envs=2, urdf=six_dof_urdf)
    assert env.spec.action_dim == 7 and env.task.scene.actuated_joints[3] == "elbow_rotate"
    env.reset(seed=0)
    ids = torch.arange(2, device=env.device)
    q0 = torch.zeros(2, 7, device=env.device)
    env.sim.set_joint_state(ids, q0, q0)
    action = torch.zeros(2, 7, device=env.device)
    action[:, 3] = 0.5  # elbow_rotate to 0.5 * 1.5 rad; position servo, so the action is a target
    for _ in range(50):
        env.sim.set_action(action)
        env.sim.step()
    q = env.sim.joint_pos()
    torch.testing.assert_close(q[:, 3], torch.full((2,), 0.75, device=env.device), atol=0.05, rtol=0)
    env.close()


def test_stock_so101_tracks_reachable_targets():
    env = elbows_sim.make("so101_pose", backend="mujoco", num_envs=8)
    env.reset(seed=0)
    for _ in range(90):  # episodes are 100 steps; stay inside the first one
        env.step((env.task.target - env.task._mid) / env.task._half)
    err = env.task._error(env.sim)
    # Some random targets put the gripper through the floor and are unreachable; most must track.
    assert (err < 0.1).sum() >= 4, err
    img = env.render(env_ids=torch.tensor([0]), width=64, height=48)
    assert img.shape == (1, 48, 64, 3) and img.float().std() > 1.0
    env.close()
