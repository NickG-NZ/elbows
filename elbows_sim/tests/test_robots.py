"""URDF robots: the stock SO-101 and arbitrary custom URDFs (e.g. an added forearm roll) load and drive."""

from __future__ import annotations

import math
import struct
from pathlib import Path

import pytest
import torch
from elbows_sim.core.actuators import DCMotor, PositionServo
from elbows_sim.formats import load_actuators, urdf_joints
from elbows_sim.robots import SO101_STOCK_URDF, so101_robot

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
    parts.append(
        _link("gripper_frame_link") + _joint("gripper_frame_joint", links[-1], "gripper_frame_link", "1 0 0", "fixed")
    )
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


def test_stock_actuators_are_sts3215():
    acts = so101_robot().actuators
    assert set(acts) == set(STOCK_JOINTS) and len(set(acts.values())) == 1
    s = acts["shoulder_pan"]
    assert isinstance(s, PositionServo)
    assert s.stall_torque == pytest.approx(30 * 9.80665 / 100, abs=1e-3)  # 30 kg·cm
    assert s.no_load_speed == pytest.approx(math.radians(60) / 0.222, abs=1e-3)  # 0.222 s/60°


SERVO = 'type = "position_servo"\nkp = 40\nstall_torque = 5\nno_load_speed = 6\n'


def test_actuator_file_per_joint(tmp_path):
    f = tmp_path / "actuators.toml"
    f.write_text(f'[default]\n{SERVO}\n[joints.gripper]\ntype = "dc_motor"\nstall_torque = 1\nno_load_speed = 9\n')
    r = so101_robot(actuators=f)
    assert r.actuators["shoulder_pan"] == PositionServo(kp=40, stall_torque=5, no_load_speed=6)
    assert r.actuators["gripper"] == DCMotor(stall_torque=1, no_load_speed=9)


@pytest.mark.parametrize(
    "body",
    [
        f"[default]\n{SERVO}gain = 2\n",  # unknown field
        '[default]\ntype = "position_servo"\nkp = 1\n',  # missing fields
        '[default]\ntype = "hydraulic"\n',  # unknown kind
        f"[default]\n{SERVO}armature = -1\n",  # negative
        f"[joints.not_a_joint]\n{SERVO}",  # joint the robot lacks
        f"[joints.gripper]\n{SERVO}",  # other joints have no actuator
    ],
)
def test_bad_actuator_files_are_rejected(tmp_path, body):
    f = tmp_path / "bad.toml"
    f.write_text(body)
    with pytest.raises(ValueError):
        load_actuators(f, list(STOCK_JOINTS))


@pytest.mark.parametrize("backend", BACKENDS)
def test_dc_motor_takes_torque_commands(backend, six_dof_urdf, tmp_path):
    """A torque-commanded joint: full action is stall torque, so an unloaded joint accelerates to
    near its no-load speed instead of holding a position."""
    f = tmp_path / "actuators.toml"
    f.write_text(f'[default]\n{SERVO}\n[joints.elbow_rotate]\ntype = "dc_motor"\nstall_torque = 2\nno_load_speed = 4\n')
    env = elbows_sim.make("so101_pose", backend=backend, num_envs=1, urdf=six_dof_urdf, actuators=f)
    env.reset(seed=0)
    j = env.task.scene.actuated_joints.index("elbow_rotate")
    q = torch.zeros(1, 7, device=env.device)
    q[0, j] = -1.5
    env.sim.set_joint_state(torch.zeros(1, dtype=torch.long, device=env.device), q, torch.zeros_like(q))
    action = torch.zeros(1, 7, device=env.device)
    action[0, j] = 1.0
    peak = 0.0
    for _ in range(25):
        env.sim.set_action(action)
        env.sim.step()
        peak = max(peak, env.sim.joint_vel()[0, j].item())
    assert 3.8 < peak < 4.01, peak  # no friction here, so it reaches the no-load speed
    env.close()


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


def test_dance_reference_and_reward_ranking():
    """Copying the reference beats doing nothing; adding the analytic wrist correction beats both."""
    env = elbows_sim.make("so101_dance", num_envs=8, max_episode_steps=60)
    names = list(env.task.scene.actuated_joints)
    assert "wrist_flex" in names and not env.task.danced[names.index("wrist_flex")]

    def run(policy) -> dict:
        env.reset(seed=0)
        for _ in range(60):
            res = env.step(policy())
        return {k: v.float().mean().item() for k, v in res.info["episode"].items()}

    def copy_ref(upright: bool = False):
        ref = env.task.reference(env.task.t + env.sim.control_dt)
        if upright:
            i = names.index
            ref[:, i("wrist_flex")] = -torch.pi / 2 - ref[:, i("shoulder_lift")] - ref[:, i("elbow_flex")]
        return env.task._normalise(ref)

    zero = run(lambda: torch.zeros(8, len(names)))
    copy = run(copy_ref)
    upright = run(lambda: copy_ref(upright=True))
    assert zero["return"] < copy["return"] < upright["return"]
    assert copy["tracking_rms"] < 0.05 and upright["upright_deg"] < 4
    env.close()


def test_dance_on_custom_urdf_dances_the_forearm_roll(six_dof_urdf):
    env = elbows_sim.make("so101_dance", num_envs=2, urdf=six_dof_urdf, max_episode_steps=10)
    names = list(env.task.scene.actuated_joints)
    assert env.task.danced[names.index("elbow_rotate")]
    obs = env.reset(seed=0)
    assert obs["policy"].shape == (2, env.task.observation_spec()["policy"].shape[0])
    for _ in range(10):
        res = env.step(torch.zeros(2, len(names)))
    assert torch.isfinite(res.reward).all() and "tracking_rms" in res.info["episode"]
    env.close()


# --- realism and safety ------------------------------------------------------------------------
HOME = [0.0, 0.0, -0.8, -0.77, 0.0, 0.3]
SELF_POSE = [-1.28, -1.48, 1.42, 1.46, -0.34, 1.69]  # folded back onto the base
FLOOR_POSE = [-0.03, 1.11, -0.12, 0.35, -0.63, 0.63]  # reaching down into the floor


@pytest.mark.parametrize("backend", BACKENDS)
def test_servo_top_speed_matches_datasheet(backend, six_dof_urdf):
    """A saturated, unloaded joint tops out at (stall - friction) / back-EMF damping, just under the
    datasheet's no-load speed (0.222 s/60 deg = 4.72 rad/s)."""
    env = elbows_sim.make("so101_pose", backend=backend, num_envs=1, urdf=six_dof_urdf)
    env.reset(seed=0)
    j = env.task.scene.actuated_joints.index("elbow_rotate")  # vertical axis: no gravity load
    q = torch.zeros(1, 7, device=env.device)
    q[0, j] = -1.5
    env.sim.set_joint_state(torch.zeros(1, dtype=torch.long, device=env.device), q, torch.zeros_like(q))
    action = torch.zeros(1, 7, device=env.device)
    action[0, j] = 1.0
    peak = 0.0
    for _ in range(25):
        env.sim.set_action(action)
        env.sim.step()
        peak = max(peak, env.sim.joint_vel()[0, j].item())
    s = env.task.scene.robot.actuators["elbow_rotate"]
    expected = (s.stall_torque - s.frictionloss) / s.back_emf_damping
    assert expected * 0.95 < peak < s.no_load_speed, (peak, expected)
    env.close()


def test_wrist_roll_hard_stop_and_command_range():
    env = elbows_sim.make("so101_pose", num_envs=1)
    j = env.task.scene.actuated_joints.index("wrist_roll")
    stop = env.sim.joint_limits()[j]
    torch.testing.assert_close(stop, torch.tensor([-163.4, 170.4]).deg2rad(), atol=1e-4, rtol=0)
    env.reset(seed=0)
    ids = torch.zeros(1, dtype=torch.long)
    q = torch.tensor([HOME])
    env.sim.set_joint_state(ids, q, torch.zeros_like(q))
    action = torch.zeros(1, 6)
    action[0, j] = 1.0  # full command: the URDF limit (+162.8 deg), inside the stop
    for _ in range(50):
        env.sim.set_action(action)
        env.sim.step()
    assert abs(math.degrees(env.sim.joint_pos()[0, j].item()) - 162.8) < 1.0
    # Thrown at the stop, the joint stops there.
    q[0, j], v = math.radians(168), torch.zeros(1, 6)
    v[0, j] = 4.0
    env.sim.set_joint_state(ids, q, v)
    peak = -math.inf
    for _ in range(10):
        env.sim.step()
        peak = max(peak, math.degrees(env.sim.joint_pos()[0, j].item()))
    assert peak < 171.0
    env.close()


def test_hard_stops_must_contain_command_range(tmp_path):
    f = tmp_path / "stops.toml"
    f.write_text("[wrist_roll]\nlower = -100\nupper = 100\n")  # inside the URDF's +-157 deg
    with pytest.raises(ValueError):
        _make_with_stops(f)


def _make_with_stops(path):
    from elbows_sim.backends.mujoco_cpu import MujocoSim
    from elbows_sim.core.spec import SceneSpec
    from elbows_sim.scenes import ARM_TABLETOP

    r = so101_robot(hard_stops=path)
    return MujocoSim(SceneSpec("x", {"mjcf": ARM_TABLETOP}, STOCK_JOINTS, robot=r), num_envs=1)


def test_robot_contacts_self_floor_and_clear():
    env = elbows_sim.make("so101_pose", num_envs=3)
    env.reset(seed=0)
    q = torch.tensor([HOME, SELF_POSE, FLOOR_POSE])
    env.sim.set_joint_state(torch.arange(3), q, torch.zeros_like(q))
    c = env.sim.robot_contacts() > 0  # [self, scene]
    assert c.tolist() == [[False, False], [True, False], [False, True]]
    env.close()


def test_dance_ends_episode_on_collision():
    env = elbows_sim.make("so101_dance", num_envs=2)
    env.reset(seed=0)
    q = torch.tensor([SELF_POSE, HOME])
    env.sim.set_joint_state(torch.arange(2), q, torch.zeros_like(q))
    res = env.step(env.task._normalise(q))
    assert res.terminated.tolist() == [True, False]
    assert res.reward[0] < -5 and res.info["episode"]["contact"].tolist() == [1.0]
    env.close()


@pytest.mark.parametrize("bpm", [(60, 120), (100, 200), (120, 100), [0, 0]])
def test_dance_rejects_unsafe_bpm(bpm):
    with pytest.raises(ValueError):
        elbows_sim.make("so101_dance", num_envs=1, bpm=bpm)


def test_dance_accepts_bpm_list():
    env = elbows_sim.make("so101_dance", num_envs=1, bpm=[100, 120])
    assert env.task.bpm == (100.0, 120.0)
    env.close()
