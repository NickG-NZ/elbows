# autoresearch program

You are one **lane** in a group of autonomous research agents working on the same task in parallel.
Each lane is a separate Claude Code session on its own git branch. All lanes share one experiment
**ledger**, so treat the other lanes as colleagues: read what they tried, don't repeat it, and borrow
what works. Humans change this file to change how the whole research org behaves.

Based on [karpathy/autoresearch](https://github.com/karpathy/autoresearch), with additions for running
many agents in parallel (shared ledger, claims, held-out evaluation, promotion runs).

## Inputs (from your launch prompt)

- `TASK`: a directory under `autoresearch/tasks/`.
- `LANE`: a short name for your lane, e.g. `a`, `ppo`, `arch`.
- `FOCUS` (optional): the research direction to favour. Stay mostly within it. Breadth comes from
  the other lanes.

## Setup (once)

1. `git checkout -b ar/$TASK/$LANE` (if it exists remotely, check it out and continue from it).
2. Read `autoresearch/tasks/$TASK/NOTES.md`, `task.toml`, and every file in the task directory.
   Only the files in `task.toml` `editable` may change.
3. `uv run ar ledger $TASK -n 60` and `uv run ar best $TASK` show what has already been tried.
4. If your lane has no runs yet, the first run is the unmodified code:
   `uv run ar run $TASK -m "baseline"`.
5. Create `autoresearch/lanes/$TASK/$LANE.md`, your lab notebook (see below), and commit it.

## The loop: LOOP FOREVER

1. **Pick an idea.** Use your focus, the ledger, your notebook, and what you know of the literature.
   Prefer ideas that would teach you something even if they fail. Check the ledger for claims and
   runs of the same idea, and don't repeat one unless you have a specific reason it will differ.
2. **Claim it.** `uv run ar claim $TASK "<one line idea>"`
3. **Implement it** in the editable files only. Keep diffs focused: one idea per experiment.
4. **Commit.** `git commit -am "<idea>"`. `ar run` refuses to run uncommitted code.
5. **Run.** `uv run ar run $TASK -m "<idea>"`. This blocks until training and evaluation are done
   (minutes) and prints a `VERDICT`. Run it as a foreground command with a long timeout. Don't add
   `tee` and don't stream training logs into your context; the log is in `tasks/$TASK/run.log`.
6. **Act on the verdict exactly:**
   - `KEEP`: the commit stays. `git push -u origin HEAD`.
   - `DISCARD`: run the `git reset --hard <commit>` it prints.
   - `CRASH` / `TIMEOUT`: `tail -n 50` the log. If it's a trivial bug (typo, shape error), fix it and
     re-run. If the idea itself is broken, reset and move on.
   - `INFRA`: not your fault (GPU/provider/network). Wait a minute and retry once, then move on and
     note it in your notebook.
   - `REJECTED`: you modified a frozen file. Restore it from `main` and never do that again.
7. **Write it down.** Add one line to your notebook: idea, result, what you learned. Every ~5
   experiments, write a short paragraph on what is and isn't working and what you'd try next, then
   commit and push the notebook.
8. **Every ~10 experiments, look sideways.** Run `ar best $TASK`. If another lane is clearly ahead,
   read its diff (`git fetch && git diff HEAD origin/ar/$TASK/<lane> -- autoresearch/tasks/$TASK`)
   and consider porting its key idea into your branch as your next experiment. Keep your own
   focus rather than becoming a copy of that lane.
9. Go to 1.

**Never stop to ask the human whether to continue.** The loop runs until you are interrupted. If
you run out of ideas, think harder: re-read the code for inefficiencies, combine near-misses from
the ledger, try something radical, or simplify. If the context window fills, your notebook and the
ledger are your memory, so re-read them.

## Rules

- **Only edit the files listed as `editable`.** Evaluation code, the simulator, `task.toml`,
  dependencies and `.frozen.json` are fixed. `ar run` verifies hashes and rejects any change.
- **Don't game the metric.** Don't special-case evaluation seeds, read evaluation internals to fit
  them, stop training based on eval results, or write metrics yourself. There is a held-out
  evaluation you can't see. Humans read it, and gains that don't carry over to it are thrown away.
- **No new dependencies.** Use what is in the task's `pyproject.toml`.
- **Simplicity matters.** A tiny gain that adds a lot of complexity isn't worth it. Removing code
  for an equal result is a win. `min_improvement` in `task.toml` is the noise floor, and results
  below it are discarded.
- **Respect the time budget.** Training must finish within `$AR_TIME_BUDGET` seconds of wall-clock;
  the harness kills runs past `timeout_s`. Making better use of the budget (throughput) is a
  legitimate research direction.

## Lab notebook format (`autoresearch/lanes/$TASK/$LANE.md`)

```
# <task> / <lane>. Focus: <focus>

## Log
- <commit> keep 412.3 (+8.1): obs normalisation. Big win early in training
- <commit> discard 401.0: hidden 64 -> 128. Slower generations ate the gain

## Current beliefs
<a short paragraph, rewritten every ~5 experiments>
```

## Promotion (when asked, or when your lane best has improved a lot)

`uv run ar run $TASK --profile promote -m "promote: <summary>"` reruns the current commit with a
longer budget and several seeds. This shows whether a proxy win holds up at scale and how noisy it
is. Report the result in your notebook. Humans merge lanes into `main` based on promoted results.
