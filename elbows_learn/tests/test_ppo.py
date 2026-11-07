import torch
from elbows_learn.rl import MLPPolicy, PPOConfig, train
from elbows_sim.evaluate import evaluate

import elbows_sim


def test_ppo_learns_reach(tmp_path):
    torch.manual_seed(0)
    env = elbows_sim.make("reach", backend="mujoco", num_envs=32, seed=0)
    rows = []
    policy = train(env, PPOConfig(total_steps=150_000), rows.append)
    returns = [r["ep_return"] for r in rows if "ep_return" in r]  # rows only have it when episodes finished
    first, best = returns[0], max(returns[-3:])
    assert best > first + 20, (first, best)

    path = tmp_path / "policy.pt"
    policy.save(path, task="reach")
    loaded, _ = MLPPolicy.load(path)
    obs = env.reset(seed=1)
    torch.testing.assert_close(loaded.act(obs), policy.act(obs))
    stats, _ = evaluate(env, loaded, episodes=8)
    assert stats["return"] > first
