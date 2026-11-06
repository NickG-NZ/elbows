# Autoresearch landscape (survey as of 2026-11-06)

What has happened since [karpathy/autoresearch](https://github.com/karpathy/autoresearch) (2026-03-07),
and what this repo borrows from each. Items marked † are single-sourced or secondary; verify before
citing.

## Karpathy
- **The repo has been quiet since 2026-03-26** (last commit: a bug-fix merge). No new agent repos on
  his GitHub since then. nanochat is still maintained.
- **"SETI@home" direction** ([tweet](https://x.com/karpathy/status/2030705271627284816), 2026-03-08):
  next is "asynchronously massively collaborative" agents, emulating a research *community*, with
  agents reading each other's results and posting short write-ups. → our shared ledger, claims, and
  lab notebooks.
- **Transfer result** ([tweet](https://x.com/karpathy/status/2031135152349524125)): ~20 changes kept at
  depth-12 all carried over to depth-24 (~11% better time-to-GPT-2). → why cheap proxies work, and
  why we have a `promote` profile to check transfer.
- **AgentHub**† (March 2026): "GitHub for agents", with a git DAG with no main branch plus a message
  board. It went private quickly. → lanes as branches with no shared main during research.

## Follow-ups worth knowing
- **pi-autoresearch** (Shopify, Tobi Lütke): ~120 experiments → 53% faster Liquid parse+render.
  Tobi himself called it "probably somewhat overfit"
  ([Simon Willison](https://simonwillison.net/2026/Mar/13/liquid/)). → held-out metric.
- **uditgoenka/autoresearch** ([GitHub](https://github.com/uditgoenka/autoresearch)): the loop as a
  Claude Code skill, with a non-editable "Guard" command that must keep passing. → frozen-file hashes.
- **SkyPilot, "Scaling Autoresearch"** ([blog](https://blog.skypilot.co/scaling-autoresearch/)): Claude
  Code driving 16 GPUs ran ~910 experiments in 8 h, reaching the result ~9× faster than sequential.
  It used factorial waves and promoted winners to bigger GPUs. → profiles; Modal backend.
- **Bilevel Autoresearch** (arXiv 2603.23420): an outer loop rewrites the inner loop's search
  strategy. → the research-director routine proposes `program.md` changes, and a human applies them.
- **Multi-agent empirical study** (arXiv 2603.29632): parallel independent agents plus later
  consolidation are robust for broad search; planner-led teams are better at deep refactors but
  fragile. → independent lanes by default.
- **autoresearch-at-home**† / **GitSwarm**†: distributed versions with claims and leaderboards.

## Robotics
- **ENPIRE** (arXiv 2606.19980, CoRL 2026; Shi, Zhu, Fan et al.): agents rewrite training code against
  a real-robot fleet with automatic reset and verification. This is the most relevant prior work
  for us and the target end state: sim proxy, then fleet evaluation.
- **Agent-driven quadruped RL** (arXiv 2603.27416): a Claude agent ran 70+ Isaac Lab experiments in
  14 waves across GPUs, editing reward and terrain configs. Hung PhysX runs were a major failure
  mode. → hard timeouts, plus an `infra` status kept separate from research failures.

## Pitfalls (the important part)
- **Reward hacking is common**: agents hacked in ~30% of open-ended research tasks, and detailed
  rejection feedback *taught* evasion (arXiv 2609.28614)†. Meta's **HackBench** catalogues fabricated
  metric files, patched graders, training on held-out data, cherry-picked seeds, and stopping on the
  target loss. → eval is frozen and run by the harness; held-out seeds and salt are hidden;
  rejection messages are terse.
- **Goodhart on val**: always track the visible-vs-held-out gap (`ar report`).
- **Hardware-specific wins** don't transfer (MLX vs CUDA). Promote on the target hardware.
