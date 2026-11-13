<h1 align="center">elbows</h1>

<p align="center">
  <strong>Understanding the sim-to-real gap, and learning to overcome it.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="Python 3.11+" src="https://img.shields.io/badge/python-3.11%2B-blue.svg">
  <img alt="Status: early development" src="https://img.shields.io/badge/status-early%20development-orange.svg">
</p>

<img width="1774" height="887" alt="Robots Exploring a Misty Mountain Wilderness" src="https://github.com/user-attachments/assets/4973c0fd-37ca-400a-bc00-12c338aa83b2" />

## Overview

Robot policies trained in simulation often fail on real hardware. The simulator's model of the
robot and its world differs from reality: actuator dynamics, friction, contact, latency, sensing.
It is rarely clear which of those differences actually matter for a given task. **elbows** has two
aims:

- **Understand the gap.** Measure where and why simulation and reality diverge, and which
  discrepancies change how a policy behaves. Use what we learn to improve the simulator, and to
  design hardware that is easier to model and control.
- **Overcome it with learning.** Develop learning-based methods that close the gap on the simulation
  side, and remain robust on the policy side. These include system identification learned from
  data, world models and neural simulators trained on real interaction, and policies that adapt
  online.

To move quickly, much of the experimentation is run by autonomous research agents. They work in
parallel against frozen evaluations, including held-out physics randomisation that acts as a proxy
for the real world.

## Repository layout

| Package | Purpose |
|---|---|
| [`elbows_sim/`](elbows_sim/README.md) | Simulator-agnostic environments, tasks and evaluation. MuJoCo today, Isaac Lab and MJWarp planned. |
| [`elbows_learn/`](elbows_learn/README.md) | Training methods and per-task recipes on top of `elbows_sim`. PPO today; VLAs, world models and neural simulators planned. |
| [`autoresearch/`](autoresearch/README.md) | Parallel autonomous research loops, after [karpathy/autoresearch](https://github.com/karpathy/autoresearch). |
| [`infra/`](infra/) | Cloud agent environment and secrets bootstrap. |
| [`docs/`](docs/) | [Agent infrastructure setup](docs/agent-infra.md) and the [autoresearch landscape survey](docs/autoresearch-landscape.md). |

## Getting started

**Prerequisites:** Python 3.11+, [uv](https://docs.astral.sh/uv/), macOS or Linux. Everything
below runs on a laptop CPU. GPU workloads run on [Modal](https://modal.com).

```bash
git clone git@github.com:NickG-NZ/elbows.git
cd elbows
uv sync
```

**Train and evaluate a policy.** PPO on a 2-link reaching arm, about a minute on a laptop CPU:

```bash
uv run elbows-learn list                                   # available recipes
uv run elbows-learn train reach_ppo                        # writes runs/reach_ppo-mujoco-s0/
uv run elbows-learn eval runs/reach_ppo-mujoco-s0/policy.pt --video runs/reach.mp4
```

**Run an autonomous research experiment.** Train, evaluate against a frozen eval, and record the
result in the shared ledger:

```bash
uv run ar tasks
uv run ar run cartpole-swingup -m "baseline"
uv run ar ledger cartpole-swingup
```

To run many research agents in parallel in the cloud, see
[`autoresearch/README.md`](autoresearch/README.md) and [`docs/agent-infra.md`](docs/agent-infra.md).

**Secrets.** Credentials (e.g. for Modal) are managed in [Doppler](https://www.doppler.com). Run
commands that need them as `doppler run -- <cmd>`. Never commit `.env` files.

## Development

```bash
uv run --group dev pytest -q                                          # tests
uv run --group dev ruff check . && uv run --group dev ruff format .   # lint and format
```

Conventions for humans and coding agents are in [`AGENTS.md`](AGENTS.md).

## Status

elbows is in early development and its interfaces will change.

- **Working today:** MuJoCo backend, a reaching task, PPO training and video evaluation, and the
  parallel autoresearch harness with a CPU cart-pole task.
- **Next:** GPU simulation (Isaac Lab, MJWarp), system identification and actuator modelling,
  world models and neural simulators, and evaluation on real hardware.

## License

Released under the [MIT License](LICENSE).
