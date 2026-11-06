"""Task spec: a directory under autoresearch/tasks/<name>/ with a task.toml."""

from __future__ import annotations

import hashlib
import json
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
TASKS_DIR = REPO_ROOT / "autoresearch" / "tasks"
LOCK_NAME = ".frozen.json"


@dataclass
class Profile:
    """How long to train and how many seeds to run. `proxy` is the default inner-loop profile."""

    budget_s: int
    timeout_s: int
    seeds: int = 1


@dataclass
class Compute:
    backend: str = "local"  # local | modal
    gpu: str | None = None  # e.g. "H100", "L40S", "A10G"
    python: str = "3.12"
    modal_secrets: list[str] = field(default_factory=list)


@dataclass
class Task:
    name: str
    dir: Path
    description: str
    editable: list[str]
    frozen: list[str]
    train_cmd: str
    eval_cmd: str
    metric: str
    goal: str  # max | min
    min_improvement: float
    profiles: dict[str, Profile]
    compute: Compute

    @classmethod
    def load(cls, name: str) -> Task:
        d = TASKS_DIR / name
        cfg = tomllib.loads((d / "task.toml").read_text())
        t, m = cfg["task"], cfg["metric"]
        profiles = {k: Profile(**v) for k, v in cfg.get("profiles", {}).items()}
        if "proxy" not in profiles:
            raise ValueError(f"{name}: task.toml must define [profiles.proxy]")
        if m["goal"] not in ("max", "min"):
            raise ValueError(f"{name}: metric.goal must be 'max' or 'min'")
        return cls(
            name=name,
            dir=d,
            description=t.get("description", ""),
            editable=t["editable"],
            frozen=t["frozen"],
            train_cmd=cfg["run"]["train"],
            eval_cmd=cfg["run"]["eval"],
            metric=m["name"],
            goal=m["goal"],
            min_improvement=float(m.get("min_improvement", 0.0)),
            profiles=profiles,
            compute=Compute(**cfg.get("compute", {})),
        )

    def dependencies(self) -> list[str]:
        return tomllib.loads((self.dir / "pyproject.toml").read_text())["project"].get("dependencies", [])

    def better(self, a: float, b: float | None) -> bool:
        """Is `a` a real improvement over `b` (beyond the noise threshold)?"""
        if b is None:
            return True
        delta = a - b if self.goal == "max" else b - a
        return delta > self.min_improvement

    # --- frozen-file integrity ------------------------------------------------
    def _hashes(self) -> dict[str, str]:
        files = [*self.frozen, "task.toml", "pyproject.toml"]
        if (self.dir / "uv.lock").exists():
            files.append("uv.lock")
        return {f: hashlib.sha256((self.dir / f).read_bytes()).hexdigest() for f in sorted(set(files))}

    def lock(self) -> None:
        (self.dir / LOCK_NAME).write_text(json.dumps(self._hashes(), indent=2) + "\n")

    def tampered(self) -> list[str]:
        lock = self.dir / LOCK_NAME
        if not lock.exists():
            raise FileNotFoundError(f"{lock} missing; run `ar lock {self.name}` on main")
        want = json.loads(lock.read_text())
        have = self._hashes()
        return sorted(f for f in want.keys() | have.keys() if want.get(f) != have.get(f))


def list_tasks() -> list[str]:
    return sorted(p.parent.name for p in TASKS_DIR.glob("*/task.toml") if not p.parent.name.startswith("_"))
