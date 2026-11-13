"""Run Physical Intelligence's pi0.5 (`pi05_libero`) on the LIBERO benchmark on Modal GPUs.

LIBERO is a Franka Panda manipulation benchmark in MuJoCo (robosuite). This reproduces openpi's
official evaluation: openpi's policy server and LIBERO client, unmodified and pinned to one commit, in
one container per task suite. openpi reports 96.85% average success (see `PUBLISHED`).

    doppler run -- uv run modal run elbows_learn/src/elbows_learn/vla/pi05_libero.py              # quick: 5 trials/task
    doppler run -- uv run modal run elbows_learn/src/elbows_learn/vla/pi05_libero.py \
        --suites libero_spatial,libero_object,libero_goal,libero_10 --trials 50                   # full benchmark

Results and sample videos are written to runs/pi05_libero/.
"""

from __future__ import annotations

import json
import os
import re
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

import modal

OPENPI_COMMIT = "215abfb217dbac7d5f1273282331b9b1866c0479"  # openpi main, 2026-08-24
GPU = "L4"  # 24 GB; inference needs >8 GB. Has graphics support for MuJoCo's EGL rendering.
PUBLISHED = {"libero_spatial": 98.8, "libero_object": 98.2, "libero_goal": 98.0, "libero_10": 92.4}
SERVER_PORT = 8000

image = (
    modal.Image.from_registry("nvidia/cuda:12.2.2-cudnn8-runtime-ubuntu22.04", add_python="3.11")
    .apt_install(
        # openpi server (scripts/docker/serve_policy.Dockerfile)
        "git",
        "git-lfs",
        "build-essential",
        "clang",
        # LIBERO client (examples/libero/Dockerfile)
        "make",
        "g++",
        "libosmesa6-dev",
        "libgl1-mesa-glx",
        "libegl1",
        "libglew-dev",
        "libglfw3-dev",
        "libgles2-mesa-dev",
        "libglib2.0-0",
        "libsm6",
        "libxrender1",
        "libxext6",
    )
    .pip_install("uv", "gsutil")  # openpi downloads its public checkpoints with gsutil
    .run_commands(
        "git clone https://github.com/Physical-Intelligence/openpi.git /openpi",
        f"cd /openpi && git checkout {OPENPI_COMMIT} && git submodule update --init --recursive third_party/libero",
        # Server: Python 3.11 + JAX, from openpi's lockfile.
        "cd /openpi && GIT_LFS_SKIP_SMUDGE=1 uv sync --frozen --no-dev",
        # Client: Python 3.8 + robosuite, exactly as openpi's LIBERO Dockerfile.
        "uv venv --python 3.8 /libero-venv",
        "cd /openpi && uv pip sync --python /libero-venv/bin/python examples/libero/requirements.txt "
        "third_party/libero/requirements.txt packages/openpi-client/pyproject.toml "
        "--extra-index-url https://download.pytorch.org/whl/cu113 --index-strategy=unsafe-best-match",
        # LIBERO prompts for a config path on first import unless this exists.
        "mkdir -p /tmp/libero && printf 'benchmark_root: /openpi/third_party/libero/libero/libero\\n"
        "bddl_files: /openpi/third_party/libero/libero/libero/bddl_files\\n"
        "init_states: /openpi/third_party/libero/libero/libero/init_files\\n"
        "datasets: /openpi/third_party/libero/libero/datasets\\n"
        "assets: /openpi/third_party/libero/libero/libero/assets\\n' > /tmp/libero/config.yaml",
        'mkdir -p /usr/share/glvnd/egl_vendor.d && echo \'{"file_format_version": "1.0.0", '
        '"ICD": {"library_path": "libEGL_nvidia.so.0"}}\' > /usr/share/glvnd/egl_vendor.d/10_nvidia.json',
    )
    .env(
        {"OPENPI_DATA_HOME": "/cache/openpi", "LIBERO_CONFIG_PATH": "/tmp/libero", "NVIDIA_DRIVER_CAPABILITIES": "all"}
    )
)

app = modal.App("elbows-pi05-libero", image=image)
cache = modal.Volume.from_name("elbows-openpi-cache", create_if_missing=True)


def _stream(proc: subprocess.Popen, prefix: str, sink: list[str]) -> threading.Thread:
    def pump() -> None:
        for line in proc.stdout:
            sink.append(line)
            print(f"[{prefix}] {line}", end="", flush=True)

    t = threading.Thread(target=pump, daemon=True)
    t.start()
    return t


def _start_server() -> tuple[subprocess.Popen, list[str]]:
    """openpi's policy server for LIBERO (pi05_libero checkpoint), on localhost:8000."""
    proc = subprocess.Popen(
        ["/openpi/.venv/bin/python", "scripts/serve_policy.py", "--env", "LIBERO"],
        cwd="/openpi",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    log: list[str] = []
    _stream(proc, "server", log)
    return proc, log


def _wait_for_port(proc: subprocess.Popen, log: list[str], timeout_s: float = 1800) -> None:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        if proc.poll() is not None:
            raise RuntimeError("policy server exited:\n" + "".join(log[-40:]))
        with socket.socket() as s:
            if s.connect_ex(("127.0.0.1", SERVER_PORT)) == 0:
                return
        time.sleep(2)
    raise TimeoutError("policy server did not start listening")


@app.function(gpu=GPU, volumes={"/cache": cache}, timeout=3600)
def prefetch() -> None:
    """Download the checkpoint and tokenizer into the shared volume once, before parallel runs."""
    proc, log = _start_server()
    try:
        _wait_for_port(proc, log)
    finally:
        proc.terminate()
    cache.commit()


@app.function(gpu=GPU, volumes={"/cache": cache}, timeout=6 * 3600)
def eval_suite(suite: str, trials: int, seed: int = 7, gl: str = "egl", max_videos: int = 4) -> dict:
    proc, server_log = _start_server()
    out_dir = Path(f"/tmp/videos/{suite}")
    env = os.environ | {
        "PYTHONPATH": "/openpi:/openpi/packages/openpi-client/src:/openpi/third_party/libero",
        "MUJOCO_GL": gl,
        "PYOPENGL_PLATFORM": gl,
        "MUJOCO_EGL_DEVICE_ID": "0",
    }
    try:
        _wait_for_port(proc, server_log)
        client = subprocess.Popen(
            [
                "/libero-venv/bin/python",
                "examples/libero/main.py",
                "--args.task-suite-name",
                suite,
                "--args.num-trials-per-task",
                str(trials),
                "--args.seed",
                str(seed),
                "--args.video-out-path",
                str(out_dir),
            ],
            cwd="/openpi",
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        client_log: list[str] = []
        pump = _stream(client, suite, client_log)
        while client.poll() is None:
            if proc.poll() is not None:  # the client retries forever if the server dies; don't hang
                client.kill()
                raise RuntimeError("policy server died mid-run:\n" + "".join(server_log[-40:]))
            time.sleep(5)
        pump.join(timeout=10)
    finally:
        proc.terminate()

    text = "".join(client_log)
    total = re.findall(r"Total success rate: ([0-9.]+)", text)
    if client.returncode != 0 or not total:
        raise RuntimeError(f"LIBERO client failed (exit {client.returncode}):\n" + "".join(client_log[-60:]))
    tasks = re.findall(r"Task: (.+)", text)
    rates = [float(x) for x in re.findall(r"Current task success rate: ([0-9.]+)", text)]
    ordered = list(dict.fromkeys(tasks))  # task description is logged once per episode
    videos = sorted(out_dir.glob("*.mp4"))
    picks = [v for v in videos if v.stem.endswith("failure")][:2] + [v for v in videos if v.stem.endswith("success")]
    return {
        "suite": suite,
        "trials_per_task": trials,
        "success_rate": float(total[-1]) * 100,
        "episodes": int(re.findall(r"Total episodes: (\d+)", text)[-1]),
        "per_task": dict(zip(ordered, (r * 100 for r in rates))),
        "videos": {v.name: v.read_bytes() for v in picks[:max_videos]},
    }


@app.local_entrypoint()
def main(
    suites: str = "libero_spatial", trials: int = 5, seed: int = 7, gl: str = "egl", out: str = "runs/pi05_libero"
) -> None:
    names = [s.strip() for s in suites.split(",") if s.strip()]
    print(f"pi05_libero on {names}, {trials} trials/task, GPU={GPU}, openpi@{OPENPI_COMMIT[:7]}")
    prefetch.remote()
    results = list(eval_suite.starmap([(s, trials, seed, gl) for s in names]))

    root = Path(out)
    for r in results:
        d = root / r["suite"]
        d.mkdir(parents=True, exist_ok=True)
        for name, data in r.pop("videos").items():
            (d / name).write_bytes(data)
    (root / "results.json").write_text(json.dumps(results, indent=2))

    print(f"\n{'suite':<16}{'ours':>8}{'published':>11}{'episodes':>10}")
    for r in results:
        pub = PUBLISHED.get(r["suite"])
        print(f"{r['suite']:<16}{r['success_rate']:>7.1f}%{(f'{pub:.1f}%' if pub else '-'):>11}{r['episodes']:>10}")
    print(f"\nresults and sample videos: {root}/", file=sys.stderr)
