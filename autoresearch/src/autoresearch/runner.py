"""Execute one experiment: train under a fixed time budget, then evaluate with frozen code.

The protocol between harness and task is deliberately tiny:
  * env in:  AR_SEED, AR_TIME_BUDGET (seconds), AR_CACHE_DIR, AR_OUT_DIR
  * eval only: AR_HOLDOUT_SALT, AR_HIDDEN_METRICS_FILE (never given to the train step)
  * stdout:  lines `METRIC <name>=<float>` are the visible metrics
  * hidden:  eval writes JSON {name: float} to $AR_HIDDEN_METRICS_FILE (held-out results)
"""

from __future__ import annotations

import io
import json
import os
import re
import statistics
import subprocess
import tarfile
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

METRIC_RE = re.compile(r"^METRIC\s+([A-Za-z0-9_.-]+)=([-+0-9.eE]+|nan|inf|-inf)\s*$", re.MULTILINE)
SKIP = {"artifacts", ".venv", "__pycache__", "run.log", ".ruff_cache"}


@dataclass
class Outcome:
    status: str  # ok | crash | timeout | infra
    metrics: dict[str, float] = field(default_factory=dict)
    hidden: dict[str, float] = field(default_factory=dict)
    seconds: float = 0.0
    log: str = ""


def _aggregate(per_seed: list[dict[str, float]]) -> dict[str, float]:
    out: dict[str, float] = {}
    for k in per_seed[0]:
        vals = [m[k] for m in per_seed if k in m]
        out[k] = statistics.fmean(vals)
        if len(vals) > 1:
            out[f"{k}_std"] = statistics.stdev(vals)
    return out


def execute(
    task_dir: str,
    train_cmd: str,
    eval_cmd: str,
    budget_s: int,
    timeout_s: int,
    seeds: int,
    cache_dir: str,
    holdout_salt: str,
    wrap: list[str],
) -> Outcome:
    """Run train+eval for each seed inside `task_dir`. Pure stdlib so it also runs inside Modal."""
    t0 = time.time()
    log_parts: list[str] = []
    visible: list[dict[str, float]] = []
    hidden: list[dict[str, float]] = []
    out_dir = Path(task_dir) / "artifacts"
    base_env = {k: v for k, v in os.environ.items() if k not in ("AR_HOLDOUT_SALT", "AR_HIDDEN_METRICS_FILE")}

    for seed in range(seeds):
        out_dir.mkdir(parents=True, exist_ok=True)
        env = {
            **base_env,
            "AR_SEED": str(seed),
            "AR_TIME_BUDGET": str(budget_s),
            "AR_CACHE_DIR": cache_dir,
            "AR_OUT_DIR": str(out_dir),
            "PYTHONUNBUFFERED": "1",
        }
        with tempfile.NamedTemporaryFile("r", suffix=".json", delete=False) as hf:
            hidden_path = hf.name
        steps = [
            ("train", train_cmd, env, timeout_s),
            ("eval", eval_cmd, {**env, "AR_HOLDOUT_SALT": holdout_salt, "AR_HIDDEN_METRICS_FILE": hidden_path}, 600),
        ]
        stdout = ""
        for name, cmd, step_env, limit in steps:
            log_parts.append(f"===== seed={seed} step={name}: {cmd}\n")
            try:
                p = subprocess.run(
                    [*wrap, "bash", "-c", cmd],
                    cwd=task_dir,
                    env=step_env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    timeout=limit,
                    check=False,
                )
            except subprocess.TimeoutExpired as e:
                out = e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
                log_parts.append(out + f"\n!!! {name} exceeded {limit}s, killed\n")
                return Outcome("timeout", seconds=time.time() - t0, log="".join(log_parts))
            log_parts.append(p.stdout)
            if p.returncode != 0:
                log_parts.append(f"\n!!! {name} exited {p.returncode}\n")
                return Outcome("crash", seconds=time.time() - t0, log="".join(log_parts))
            if name == "eval":
                stdout = p.stdout
        m = {k: float(v) for k, v in METRIC_RE.findall(stdout)}
        if not m:
            log_parts.append("\n!!! eval printed no METRIC lines\n")
            return Outcome("crash", seconds=time.time() - t0, log="".join(log_parts))
        visible.append(m)
        try:
            hidden.append(json.loads(Path(hidden_path).read_text() or "{}"))
        except json.JSONDecodeError:
            hidden.append({})
        finally:
            Path(hidden_path).unlink(missing_ok=True)

    return Outcome(
        "ok",
        metrics=_aggregate(visible),
        hidden=_aggregate(hidden) if hidden and hidden[0] else {},
        seconds=time.time() - t0,
        log="".join(log_parts),
    )


def bundle(task_dir: Path) -> bytes:
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for p in sorted(task_dir.rglob("*")):
            rel = p.relative_to(task_dir)
            if p.is_file() and not (set(rel.parts) & SKIP):
                tar.add(p, arcname=str(rel))
    return buf.getvalue()
