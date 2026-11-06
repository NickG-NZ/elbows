"""Run an experiment on a Modal GPU container.

Claude Code cloud sessions have no GPU (4 vCPU / 16 GB), so they act as the "researcher" and ship
each experiment here. The held-out salt comes from the Modal secret `autoresearch-holdout`, which
lives only inside the container: the agent's own environment never sees it.
"""

from __future__ import annotations

from .runner import Outcome
from .task import Profile, Task

CACHE_VOLUME = "autoresearch-cache"
HOLDOUT_SECRET = "autoresearch-holdout"


def run(task: Task, profile: Profile, payload: bytes) -> Outcome:
    import modal

    image = (
        modal.Image.debian_slim(python_version=task.compute.python)
        .pip_install(*task.dependencies())
        .add_local_python_source("autoresearch")
    )
    app = modal.App(f"autoresearch-{task.name}")
    secrets = [modal.Secret.from_name(HOLDOUT_SECRET), *(modal.Secret.from_name(s) for s in task.compute.modal_secrets)]
    volume = modal.Volume.from_name(CACHE_VOLUME, create_if_missing=True)

    @app.function(
        image=image,
        gpu=task.compute.gpu,
        timeout=profile.seeds * (profile.timeout_s + 600) + 300,
        volumes={"/cache": volume},
        secrets=secrets,
        serialized=True,
    )
    def remote(payload: bytes, train_cmd: str, eval_cmd: str, budget_s: int, timeout_s: int, seeds: int) -> dict:
        import dataclasses
        import io
        import os
        import tarfile

        from autoresearch.runner import execute

        os.makedirs("/root/task", exist_ok=True)
        with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as tar:
            tar.extractall("/root/task", filter="data")
        salt = os.environ.pop("AR_HOLDOUT_SALT", "")
        out = execute("/root/task", train_cmd, eval_cmd, budget_s, timeout_s, seeds, "/cache", salt, wrap=[])
        volume.commit()
        return dataclasses.asdict(out)

    with app.run():
        res = remote.remote(payload, task.train_cmd, task.eval_cmd, profile.budget_s, profile.timeout_s, profile.seeds)
    return Outcome(**res)
