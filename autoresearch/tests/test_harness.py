import json
import sys
import textwrap
from pathlib import Path

from autoresearch.runner import execute
from autoresearch.task import Task


def _task(tmp: Path, train: str, eval_: str) -> Path:
    (tmp / "train.py").write_text(textwrap.dedent(train))
    (tmp / "eval.py").write_text(textwrap.dedent(eval_))
    return tmp


EVAL = """
import json, os
print("METRIC score=" + open(os.path.join(os.environ["AR_OUT_DIR"], "x")).read())
json.dump({"held": float(len(os.environ["AR_HOLDOUT_SALT"]))}, open(os.environ["AR_HIDDEN_METRICS_FILE"], "w"))
"""


def run(d: Path, timeout: int = 30, seeds: int = 1):
    py = sys.executable
    return execute(str(d), f"{py} train.py", f"{py} eval.py", 5, timeout, seeds, str(d), "salty", wrap=[])


def test_ok_and_hidden_isolation(tmp_path):
    d = _task(
        tmp_path,
        """
        import os
        assert "AR_HOLDOUT_SALT" not in os.environ, "train step must not see the salt"
        open(os.path.join(os.environ["AR_OUT_DIR"], "x"), "w").write(str(1 + int(os.environ["AR_SEED"])))
    """,
        EVAL,
    )
    out = run(d, seeds=2)
    assert out.status == "ok", out.log
    assert out.metrics["score"] == 1.5 and "score_std" in out.metrics
    assert out.hidden["held"] == 5.0
    assert "salty" not in out.log


def test_crash_and_timeout(tmp_path):
    assert run(_task(tmp_path, "raise SystemExit(1)", EVAL)).status == "crash"
    assert run(_task(tmp_path, "import time; time.sleep(10)", EVAL), timeout=1).status == "timeout"


def test_better_respects_noise_floor():
    t = Task.load("cartpole-swingup")
    assert t.better(400.0, None)
    assert not t.better(401.0, 400.0)  # below min_improvement
    assert t.better(403.0, 400.0)


def test_cartpole_lock_is_current():
    assert Task.load("cartpole-swingup").tampered() == []
    assert json.loads((Task.load("cartpole-swingup").dir / ".frozen.json").read_text())
