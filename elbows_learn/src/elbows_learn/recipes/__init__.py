"""All recipes, by name. Add a module per task (or family of tasks) and register its recipes here."""

from .base import Recipe
from .reach_ppo import REACH_PPO, REACH_PPO_SMOKE
from .so101_dance_ppo import SO101_DANCE_PPO, SO101_DANCE_PPO_SMOKE

RECIPES: dict[str, Recipe] = {r.name: r for r in (REACH_PPO, REACH_PPO_SMOKE, SO101_DANCE_PPO, SO101_DANCE_PPO_SMOKE)}

__all__ = ["RECIPES", "Recipe"]
