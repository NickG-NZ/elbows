"""PPO from proprioceptive state on elbows_sim's `reach` task (2-link arm, random targets)."""

from ..rl import PPOConfig
from .base import Recipe

REACH_PPO = Recipe(
    name="reach_ppo",
    task="reach",
    description="PPO from state. ~1 min on a laptop CPU (MuJoCo); 60-90% success depending on eval seed.",
    num_envs=64,
    ppo=PPOConfig(
        total_steps=1_000_000,
        rollout_steps=32,
        epochs=5,
        minibatches=4,
        lr=3e-4,
        gamma=0.99,
        hidden=(128, 128),
        init_log_std=-0.5,
    ),
)

# Same method, tiny budget: checks the whole pipeline end to end in ~10 s. Won't solve the task.
REACH_PPO_SMOKE = Recipe(
    name="reach_ppo_smoke",
    task="reach",
    description="Pipeline smoke test: 100k steps, short episodes. Does not solve the task.",
    num_envs=32,
    task_kwargs={"max_episode_steps": 50},
    eval_episodes=8,
    ppo=PPOConfig(total_steps=100_000, hidden=(64, 64)),
)
