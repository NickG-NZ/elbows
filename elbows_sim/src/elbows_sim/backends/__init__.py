"""Backend registry. Backends are imported lazily so a missing simulator never breaks the others."""

from __future__ import annotations

import importlib

import torch

from ..core.sim import Sim
from ..core.spec import SceneSpec

BACKENDS: dict[str, str] = {
    "mujoco": "elbows_sim.backends.mujoco_cpu:MujocoSim",
    "isaac": "elbows_sim.backends.isaac:IsaacLabSim",
    # "mjwarp": "elbows_sim.backends.mjwarp:MjWarpSim",  # GPU MuJoCo, same interface
}


def backend_class(name: str) -> type[Sim]:
    if name not in BACKENDS:
        raise KeyError(f"unknown backend {name!r}; choose from {sorted(BACKENDS)}")
    module, cls = BACKENDS[name].split(":")
    try:
        return getattr(importlib.import_module(module), cls)
    except ImportError as e:
        raise ImportError(f"backend {name!r} is not installed ({e}); try `uv sync --extra {name}`") from e


def available_backends() -> list[str]:
    out = []
    for name in BACKENDS:
        try:
            if backend_class(name).available():
                out.append(name)
        except ImportError:
            pass
    return out


def make_sim(name: str, scene: SceneSpec, num_envs: int, device: str | torch.device | None = None) -> Sim:
    return backend_class(name)(scene, num_envs=num_envs, device=device)
