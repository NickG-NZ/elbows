"""Shared, append-only experiment ledger.

Every run (and every claim of an idea about to be tried) is one JSON record. Parallel lanes read
each other's records to avoid duplicate work and to borrow wins.

Backends, picked by $AR_LEDGER:
  local (default) -> autoresearch/.ledger/<task>.jsonl (fine for one machine / git worktrees)
  modal           -> a Modal Dict named "autoresearch-ledger" (shared across cloud sessions)
"""

from __future__ import annotations

import json
import os
import time
import uuid

from .task import REPO_ROOT

LOCAL_DIR = REPO_ROOT / "autoresearch" / ".ledger"
MODAL_DICT = "autoresearch-ledger"

# Fields never shown to agents by default (held-out metrics live here).
HIDDEN = "hidden"


def new_id() -> str:
    return time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:6]


class Ledger:
    def __init__(self, task: str, backend: str | None = None):
        self.task = task
        self.backend = backend or os.environ.get("AR_LEDGER", "local")
        if self.backend not in ("local", "modal"):
            raise ValueError(f"unknown AR_LEDGER backend {self.backend!r}")

    def _dict(self):
        import modal

        return modal.Dict.from_name(MODAL_DICT, create_if_missing=True)

    def append(self, rec: dict) -> dict:
        rec = {"id": new_id(), "task": self.task, "ts": time.time(), **rec}
        if self.backend == "local":
            LOCAL_DIR.mkdir(parents=True, exist_ok=True)
            with open(LOCAL_DIR / f"{self.task}.jsonl", "a") as f:
                f.write(json.dumps(rec) + "\n")
        else:
            self._dict()[f"{self.task}/{rec['id']}"] = rec
        return rec

    def records(self) -> list[dict]:
        if self.backend == "local":
            p = LOCAL_DIR / f"{self.task}.jsonl"
            recs = [json.loads(line) for line in p.read_text().splitlines() if line.strip()] if p.exists() else []
        else:
            d = self._dict()
            prefix = f"{self.task}/"
            recs = [v for k, v in d.items() if k.startswith(prefix)]
        return sorted(recs, key=lambda r: r["ts"])

    def runs(self, lane: str | None = None, profile: str | None = "proxy") -> list[dict]:
        return [
            r
            for r in self.records()
            if r.get("kind") == "run"
            and (lane is None or r.get("lane") == lane)
            and (profile is None or r.get("profile") == profile)
        ]

    def best(self, metric: str, goal: str, lane: str | None = None) -> dict | None:
        kept = [r for r in self.runs(lane) if r.get("status") == "keep" and metric in r.get("metrics", {})]
        if not kept:
            return None
        pick = max if goal == "max" else min
        return pick(kept, key=lambda r: r["metrics"][metric])
