"""Contract every `Sim` backend must satisfy. New backends get these tests for free via `available_backends()`."""

from __future__ import annotations

import math

import pytest
import torch
from elbows_sim.tasks.reach import LINK_LENGTHS

import elbows_sim

BACKENDS = elbows_sim.available_backends()
N = 4


@pytest.fixture(params=BACKENDS)
def backend(request) -> str:
    return request.param


@pytest.fixture
def env(backend):
    e = elbows_sim.make("reach", backend=backend, num_envs=N)
    yield e
    e.close()


def test_spec_and_shapes(env):
    obs = env.reset(seed=0)
    for k, spec in env.spec.observations.items():
        assert obs[k].shape == (N, *spec.shape) and obs[k].dtype == spec.dtype
        assert obs[k].device.type == env.device.type
    res = env.step(torch.zeros(N, env.spec.action_dim))
    assert res.reward.shape == res.terminated.shape == res.truncated.shape == (N,)
    assert res.terminated.dtype == torch.bool and torch.isfinite(res.reward).all()


def test_reset_is_seeded(backend):
    a, b = (elbows_sim.make("reach", backend=backend, num_envs=N) for _ in range(2))
    torch.testing.assert_close(a.reset(seed=7)["policy"], b.reset(seed=7)["policy"])
    assert not torch.allclose(a.reset(seed=7)["policy"], a.reset(seed=8)["policy"])


def test_joint_state_roundtrip_and_kinematics(env):
    """Same scene, same names, same kinematics on every backend: env-local tip position = analytic FK."""
    env.reset(seed=0)
    sim, ids = env.sim, torch.arange(N, device=env.device)
    q = torch.tensor([[0.0, 0.0], [math.pi / 2, 0.0], [0.3, -1.2], [-2.0, 2.0]], device=env.device)
    sim.set_joint_state(ids, q, torch.zeros_like(q))
    torch.testing.assert_close(sim.joint_pos(), q)
    torch.testing.assert_close(sim.joint_vel(), torch.zeros_like(q))
    l1, l2 = LINK_LENGTHS
    expected = torch.stack(
        [l1 * q[:, 0].cos() + l2 * (q[:, 0] + q[:, 1]).cos(), l1 * q[:, 0].sin() + l2 * (q[:, 0] + q[:, 1]).sin()],
        dim=-1,
    )
    torch.testing.assert_close(sim.frame_pos("tip")[:, :2], expected, atol=1e-5, rtol=0)


def test_zero_action_is_zero_torque(env):
    env.reset(seed=0)
    ids = torch.arange(N, device=env.device)
    q = torch.full((N, 2), 0.5, device=env.device)
    env.sim.set_joint_state(ids, q, torch.zeros_like(q))
    for _ in range(10):
        env.sim.set_action(torch.zeros(N, 2, device=env.device))
        env.sim.step()
    torch.testing.assert_close(env.sim.joint_pos(), q, atol=1e-4, rtol=0)


def test_positive_action_accelerates_that_joint_positively(env):
    """Action i drives joint i, with the sign convention of the scene (checked one joint at a time)."""
    ids = torch.arange(N, device=env.device)
    for j in range(env.spec.action_dim):
        env.reset(seed=0)
        q = torch.zeros(N, env.spec.action_dim, device=env.device)
        env.sim.set_joint_state(ids, q, q)
        a = torch.zeros_like(q)
        a[:, j] = 1.0
        env.sim.set_action(a)
        env.sim.step()
        assert (env.sim.joint_vel()[:, j] > 0).all()


def test_joint_limits(env):
    lim = env.sim.joint_limits()
    assert lim.shape == (2, 2)
    assert torch.isinf(lim[0]).all()  # shoulder is continuous
    torch.testing.assert_close(lim[1].cpu(), torch.tensor([-150.0, 150.0]).deg2rad(), atol=1e-5, rtol=0)


def test_marker_and_target_agree(env):
    env.reset(seed=3)
    target = env.task.target[:, :2]
    obs = env.reset(seed=3)["policy"]
    torch.testing.assert_close(obs[:, 8:10], target)  # obs: cos q, sin q, qd, tip, target, diff


def test_auto_reset_and_episode_info(backend):
    env = elbows_sim.make("reach", backend=backend, num_envs=N, max_episode_steps=5)
    env.reset(seed=0)
    for i in range(5):
        res = env.step(torch.zeros(N, 2))
    assert res.truncated.all() and not res.terminated.any()
    ep = res.info["episode"]
    assert set(ep) >= {"return", "length", "success", "final_distance"}
    assert (ep["length"] == 5).all() and (env.episode_len == 0).all()


def test_render(env):
    env.reset(seed=0)
    img = env.render(env_ids=torch.tensor([0, 2]), width=64, height=48)
    assert img.shape == (2, 48, 64, 3) and img.dtype == torch.uint8
    assert img.float().std() > 1.0  # not a blank frame
