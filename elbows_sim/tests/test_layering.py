"""Dependency rules: elbows_sim never imports training code, and only backends import simulators."""

from __future__ import annotations

import ast
from pathlib import Path

SRC = Path(__file__).parents[1] / "src" / "elbows_sim"
SIMULATORS = {"mujoco", "mujoco_warp", "warp", "isaaclab", "isaacsim", "omni", "pxr"}


def imports(path: Path) -> set[str]:
    tree = ast.parse(path.read_text())
    out = {a.name.split(".")[0] for n in ast.walk(tree) if isinstance(n, ast.Import) for a in n.names}
    out |= {
        n.module.split(".")[0] for n in ast.walk(tree) if isinstance(n, ast.ImportFrom) and n.module and not n.level
    }
    return out


def test_sim_never_imports_learn():
    offenders = [p for p in SRC.rglob("*.py") if "elbows_learn" in imports(p)]
    assert not offenders, offenders


def test_only_backends_import_simulators():
    offenders = [p for p in SRC.rglob("*.py") if "backends" not in p.parts and imports(p) & SIMULATORS]
    assert not offenders, offenders
