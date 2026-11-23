"""PPO on elbows_sim's `so101_dance`: track a beat-synchronised reference with the gripper upright.

Reference points on the stock arm (64 envs, 250-step episodes, return per episode):
zero action ~2, copying the reference ~223 (gripper ~10 deg off vertical), copying it with an
analytic wrist correction ~244 (~2 deg). The policy must learn the wrist correction itself.

For a modified arm: `elbows-learn train so101_dance_ppo --task-arg urdf=path/to/arm.urdf`.
"""

from ..rl import PPOConfig
from .base import Recipe

SO101_DANCE_PPO = Recipe(
    name="so101_dance_ppo",
    task="so101_dance",
    description="SO-101 dance from state: follow a beat reference, keep the gripper up.",
    num_envs=64,
    ppo=PPOConfig(
        total_steps=3_000_000,
        rollout_steps=32,
        epochs=5,
        minibatches=4,
        lr=3e-4,
        gamma=0.99,
        hidden=(256, 256),
        init_log_std=-1.0,  # actions are target positions; start with modest exploration
    ),
)

SO101_DANCE_PPO_SMOKE = Recipe(
    name="so101_dance_ppo_smoke",
    task="so101_dance",
    description="Pipeline smoke test: 50k steps, short episodes. Does not learn to dance.",
    num_envs=16,
    task_kwargs={"max_episode_steps": 50},
    eval_episodes=4,
    ppo=PPOConfig(total_steps=50_000, hidden=(64, 64), init_log_std=-1.0),
)
