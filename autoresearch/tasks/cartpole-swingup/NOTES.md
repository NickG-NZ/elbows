# cartpole-swingup: task notes

Read alongside `autoresearch/program.md`. Humans own this file.

- Goal: maximise `val_return` (cap 500) from a policy trained in 60 s on CPU.
- Contract: `train.py` must write a TorchScript module to `$AR_OUT_DIR/policy.pt`, mapping obs `[n, 5]`
  to action `[n, 1]` in [-1, 1]. Only `train.py` may change; deps are fixed by `pyproject.toml`.
- Evaluation randomises masses, pole length and friction. A held-out set you cannot see uses wider
  randomisation. Robust policies win there; policies tuned to the val seeds lose.
- Ideas worth trying: energy-based swing-up curriculum, PPO / SAC instead of CEM, larger networks,
  observation normalisation, domain randomisation during training, antithetic sampling, and better
  use of the time budget (e.g. vectorise more episodes).
