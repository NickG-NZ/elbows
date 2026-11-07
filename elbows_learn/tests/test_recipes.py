import dataclasses

import torch
from elbows_learn.recipes import RECIPES
from elbows_learn.rl import train

from elbows_sim import TASKS, available_backends, make


def test_recipes_point_at_real_tasks_and_names_match():
    for name, r in RECIPES.items():
        assert r.name == name
        assert r.task in TASKS, f"{name}: unknown task {r.task!r}"


def test_every_recipe_runs_one_iteration():
    """Each recipe builds its env and completes one PPO iteration (budget shrunk to keep this fast)."""
    for r in RECIPES.values():
        if r.backend not in available_backends():
            continue
        num_envs = 4
        ppo = dataclasses.replace(r.ppo, total_steps=num_envs * r.ppo.rollout_steps)
        torch.manual_seed(0)
        env = make(r.task, backend=r.backend, num_envs=num_envs, **r.task_kwargs)
        rows = []
        policy = train(env, ppo, rows.append)
        assert len(rows) == 1
        assert policy.act(env.reset(seed=0)).shape == (num_envs, env.spec.action_dim)
        env.close()
