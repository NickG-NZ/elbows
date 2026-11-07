# autoresearch

Many agents run autonomous research loops in parallel. This is
[karpathy/autoresearch](https://github.com/karpathy/autoresearch) scaled from one agent on one GPU to
a small research org:

| karpathy/autoresearch           | here                                                              |
|---------------------------------|-------------------------------------------------------------------|
| one agent, one branch           | N **lanes** (`ar/<task>/<lane>` branches), each a cloud session   |
| `results.tsv` per run           | shared **ledger** across lanes, plus **claims** to avoid duplicates |
| `train.py` + `prepare.py`       | any **task** dir: `task.toml` + editable files + frozen eval        |
| "don't touch prepare.py"        | frozen files are **hash-checked** against `main`; runs are rejected |
| val_bpb only                    | visible metric + **held-out metric** the agents never see           |
| local GPU                       | `local` backend, or the **`modal`** backend (GPU containers)        |
| 5-min runs                      | `proxy` profile in the loop, `promote` profile (longer, multi-seed) |

`program.md` is the "research org code" that every lane follows. Edit it to change how they all
behave. `prompts/review.md` is the research director: a daily routine that reads the ledger and
the held-out gap, flags overfitting and metric gaming, and recommends promotions and merges.

## Quick start (local)

```bash
uv sync
uv run ar tasks
uv run ar run cartpole-swingup -m "baseline"     # ~70 s on a laptop CPU
uv run ar ledger cartpole-swingup
autoresearch/scripts/launch.sh --local cartpole-swingup "a:improve CEM" "b:try PPO"   # worktree + tmux per lane
```

## Parallel in the cloud

See [`docs/agent-infra.md`](../docs/agent-infra.md) for the one-time setup (Claude Code cloud environment,
Doppler, Modal). After that:

```bash
git push
autoresearch/scripts/launch.sh cartpole-swingup \
  "cem:improve the CEM baseline" "ppo:replace CEM with batched PPO" "robust:robustness to physics noise"
```

Each lane is a `claude --cloud` session. Experiments go to Modal GPUs when the task says
`backend = "modal"` or `AR_BACKEND=modal`, and lanes share the ledger when `AR_LEDGER=modal`.
Claude Code cloud VMs are 4 vCPU / 16 GB with no GPU.

## The `ar` CLI

| command | who | what |
|---|---|---|
| `ar run <task> -m "idea" [--profile promote]` | agent | verify frozen files → train (time budget) → eval → record → print `VERDICT` |
| `ar claim <task> "idea"` | agent | announce an idea so other lanes skip it |
| `ar ledger <task> [-n 40] [--lane x]` | agent | recent runs and claims across lanes |
| `ar best <task>` | agent | best kept run per lane |
| `ar report <task>` | human / director | visible vs held-out metric per kept run (the overfitting alarm) |
| `ar lock <task>` | human | record hashes of frozen files; commit `.frozen.json` on `main` |

## Adding a task

Create `tasks/<name>/` with:

- `task.toml`: `editable`, `frozen`, `run.train`, `run.eval`, `metric` (`name`, `goal`,
  `min_improvement` set to about 2× the seed noise), `profiles.proxy` (+ `promote`), and `compute`
  (`backend`, `gpu`, `modal_secrets`).
- `pyproject.toml`: pinned deps (`uv lock` inside the dir). Agents cannot change these.
- the editable training code, which reads `AR_TIME_BUDGET`, `AR_SEED`, `AR_OUT_DIR`, `AR_CACHE_DIR`.
- a frozen eval that prints `METRIC <name>=<value>` and writes held-out metrics as JSON to
  `$AR_HIDDEN_METRICS_FILE`, seeded from `$AR_HOLDOUT_SALT`. Only the eval step receives these two.
- `NOTES.md`: task-specific context and ideas for the agents.

Then `uv run ar lock <name>`, commit on `main`, and run a baseline before launching lanes.

Good robotics tasks for this loop have a cheap proxy that predicts the expensive result: a short
sim training budget (MuJoCo Playground / MJX, Isaac Lab, ManiSkill) whose ranking holds at longer
budgets, and later at real-robot evaluation. Check that with `--profile promote` before trusting
a task. Karpathy's depth-12 → depth-24 transfer result is the reason the method works at all.

## Threat model (honest version)

- **Frozen-file tampering** is caught by hashes checked against `main`'s lockfile.
- **The held-out salt** is truly hidden only on the Modal backend, where it lives in a Modal secret
  inside the GPU container. On the local backend it's an env var: honour system.
- **Held-out metrics** are stored in the ledger under `hidden`. `ar ledger` hides them, but an agent
  with the Modal token could read the Dict. Agents rarely go looking, and the director review
  compares diffs to the held-out gap. If that becomes a problem, move them to a store that agent
  credentials can't read.
- **Metric gaming in the editable code** (special-casing seeds, stopping early based on eval) can't be
  prevented mechanically. The held-out gap and the director's diff review are how you detect it.
