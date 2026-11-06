"""`ar` — the autoresearch command line. Agents use `run`, `claim`, `ledger`, `best`; humans also use `report`."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

from . import runner
from .ledger import Ledger
from .task import REPO_ROOT, Task, list_tasks


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=REPO_ROOT, capture_output=True, text=True, check=True).stdout.strip()


def current_lane(task: str) -> str:
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    prefix = f"ar/{task}/"
    if branch.startswith(prefix):
        return branch[len(prefix) :]
    return os.environ.get("AR_LANE", "local")


def fmt(v: float | None) -> str:
    return "-" if v is None else f"{v:.6g}"


def cmd_run(a: argparse.Namespace) -> int:
    task = Task.load(a.task)
    profile = task.profiles[a.profile]
    backend = a.backend or os.environ.get("AR_BACKEND") or task.compute.backend
    ledger = Ledger(task.name)
    lane = current_lane(task.name)

    dirty = git("status", "--porcelain", "--", str(task.dir))
    if dirty and not a.allow_dirty:
        print(f"Commit your change first (the commit hash identifies the experiment):\n{dirty}")
        return 2
    commit = git("rev-parse", "--short=8", "HEAD")
    base = {
        "kind": "run",
        "lane": lane,
        "commit": commit,
        "branch": git("rev-parse", "--abbrev-ref", "HEAD"),
        "description": a.message,
        "profile": a.profile,
        "backend": backend,
    }

    tampered = task.tampered()
    lock_rel = (task.dir / ".frozen.json").relative_to(REPO_ROOT)
    for ref in ("origin/main", "main"):
        try:
            if git("show", f"{ref}:{lock_rel}") != (task.dir / ".frozen.json").read_text().strip():
                tampered.append(f".frozen.json (differs from {ref})")
            break
        except subprocess.CalledProcessError:
            continue
    if tampered:
        ledger.append({**base, "status": "rejected", "metrics": {}})
        print(
            f"REJECTED: frozen files differ from {task.dir.name}/.frozen.json: {', '.join(tampered)}\n"
            f"Restore them (git checkout main -- <file>) and do not edit them."
        )
        return 3

    if backend == "modal":
        from . import modal_backend

        try:
            out = modal_backend.run(task, profile, runner.bundle(task.dir))
        except Exception as e:  # noqa: BLE001 - infra failures are logged separately from research failures
            out = runner.Outcome("infra", log=f"modal error: {e!r}")
    else:
        cache = os.environ.get("AR_CACHE_DIR", str(Path.home() / ".cache" / "autoresearch" / task.name))
        Path(cache).mkdir(parents=True, exist_ok=True)
        out = runner.execute(
            str(task.dir),
            task.train_cmd,
            task.eval_cmd,
            profile.budget_s,
            profile.timeout_s,
            profile.seeds,
            cache,
            os.environ.get("AR_HOLDOUT_SALT", "local-dev-salt"),
            wrap=["uv", "run", "--quiet", "--project", str(task.dir)],
        )
    (task.dir / "run.log").write_text(out.log)

    value = out.metrics.get(task.metric)
    prev = ledger.best(task.metric, task.goal, lane)
    prev_v = prev["metrics"][task.metric] if prev else None
    if out.status != "ok" or value is None:
        status = out.status if out.status != "ok" else "crash"
    elif a.profile != "proxy":
        status = a.profile
    else:
        status = "keep" if task.better(value, prev_v) else "discard"
    rec = ledger.append(
        {**base, "status": status, "metrics": out.metrics, "hidden": out.hidden, "seconds": round(out.seconds, 1)}
    )

    glob = ledger.best(task.metric, task.goal)
    print(f"run {rec['id']}  task={task.name} lane={lane} commit={commit} profile={a.profile} backend={backend}")
    print("  " + "  ".join(f"{k}={fmt(v)}" for k, v in out.metrics.items()) or "  (no metrics)")
    print(
        f"  lane best before: {fmt(prev_v)}   global best: {fmt(glob['metrics'][task.metric]) if glob else '-'}"
        + (f" (lane {glob['lane']} @ {glob['commit']})" if glob else "")
    )
    if status == "keep":
        print("VERDICT: KEEP. The commit stays. Push it: git push -u origin HEAD")
    elif status == "discard":
        target = prev["commit"] if prev else "HEAD~1"
        print(
            f"VERDICT: DISCARD (needs > {task.min_improvement} better than lane best). Revert: git reset --hard {target}"
        )
    elif status in ("crash", "timeout", "infra"):
        print(f"VERDICT: {status.upper()}. See `tail -n 50 {task.dir.relative_to(REPO_ROOT)}/run.log`.")
    else:
        print(f"VERDICT: recorded as {status}.")
    return 0


def cmd_claim(a: argparse.Namespace) -> int:
    Ledger(a.task).append({"kind": "claim", "lane": current_lane(a.task), "description": a.idea})
    print("claimed")
    return 0


def cmd_ledger(a: argparse.Namespace) -> int:
    task = Task.load(a.task)
    recs = Ledger(task.name).records()
    if a.lane:
        recs = [r for r in recs if r.get("lane") == a.lane]
    cols = ["id", "lane", "kind", "commit", "status", task.metric] + (["hidden"] if a.reveal else []) + ["description"]
    print("\t".join(cols))
    for r in recs[-a.n :]:
        row = [
            r["id"],
            r.get("lane", ""),
            r["kind"],
            r.get("commit", ""),
            r.get("status", ""),
            fmt(r.get("metrics", {}).get(task.metric)),
        ]
        if a.reveal:
            row.append(" ".join(f"{k}={fmt(v)}" for k, v in r.get("hidden", {}).items()))
        print("\t".join([*row, r.get("description", "")]))
    return 0


def cmd_best(a: argparse.Namespace) -> int:
    task = Task.load(a.task)
    ledger = Ledger(task.name)
    lanes = sorted({r["lane"] for r in ledger.runs()})
    rows = [b for lane in lanes if (b := ledger.best(task.metric, task.goal, lane))]
    rows.sort(key=lambda r: r["metrics"][task.metric], reverse=task.goal == "max")
    print(f"lane\tcommit\tbranch\t{task.metric}\tdescription")
    for r in rows:
        print(f"{r['lane']}\t{r['commit']}\t{r['branch']}\t{fmt(r['metrics'][task.metric])}\t{r['description']}")
    return 0


def cmd_report(a: argparse.Namespace) -> int:
    """Human-only: visible vs held-out metric for every kept run. A widening gap means overfitting."""
    task = Task.load(a.task)
    print(f"lane\tcommit\t{task.metric}\theld-out\tgap\tdescription")
    for r in Ledger(task.name).runs(profile=None):
        if r["status"] not in ("keep", "promote"):
            continue
        v = r["metrics"].get(task.metric)
        h = next(iter(r.get("hidden", {}).values()), None)
        gap = None if v is None or h is None else v - h
        print(f"{r['lane']}\t{r['commit']}\t{fmt(v)}\t{fmt(h)}\t{fmt(gap)}\t{r['description']}")
    return 0


def cmd_lock(a: argparse.Namespace) -> int:
    Task.load(a.task).lock()
    print(f"wrote {a.task}/.frozen.json, commit it on main")
    return 0


def cmd_tasks(_: argparse.Namespace) -> int:
    for name in list_tasks():
        t = Task.load(name)
        print(f"{name}\t{t.metric} ({t.goal})\t{t.compute.backend}\t{t.description}")
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="ar", description=__doc__)
    sub = p.add_subparsers(required=True)

    s = sub.add_parser("run", help="run one experiment on the committed code and record a verdict")
    s.add_argument("task")
    s.add_argument("-m", "--message", required=True, help="one-line description of the idea")
    s.add_argument("--profile", default="proxy", help="proxy (inner loop) or a promotion profile from task.toml")
    s.add_argument("--backend", choices=["local", "modal"])
    s.add_argument("--allow-dirty", action="store_true")
    s.set_defaults(fn=cmd_run)

    s = sub.add_parser("claim", help="announce an idea you are about to try so other lanes skip it")
    s.add_argument("task")
    s.add_argument("idea")
    s.set_defaults(fn=cmd_claim)

    s = sub.add_parser("ledger", help="recent runs and claims across all lanes")
    s.add_argument("task")
    s.add_argument("-n", type=int, default=40)
    s.add_argument("--lane")
    s.add_argument("--reveal", action="store_true", help=argparse.SUPPRESS)
    s.set_defaults(fn=cmd_ledger)

    s = sub.add_parser("best", help="best kept run per lane")
    s.add_argument("task")
    s.set_defaults(fn=cmd_best)

    s = sub.add_parser("report", help=argparse.SUPPRESS)
    s.add_argument("task")
    s.set_defaults(fn=cmd_report)

    s = sub.add_parser("lock", help="(human) record hashes of frozen files")
    s.add_argument("task")
    s.set_defaults(fn=cmd_lock)

    sub.add_parser("tasks", help="list tasks").set_defaults(fn=cmd_tasks)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
