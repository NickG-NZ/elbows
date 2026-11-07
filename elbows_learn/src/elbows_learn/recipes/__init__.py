"""All recipes, by name. Add a module per task (or family of tasks) and register its recipes here."""

from .base import Recipe
from .reach_ppo import REACH_PPO, REACH_PPO_SMOKE

RECIPES: dict[str, Recipe] = {r.name: r for r in (REACH_PPO, REACH_PPO_SMOKE)}

__all__ = ["RECIPES", "Recipe"]
