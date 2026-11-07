# Running many agents: setup runbook

The setup has three layers:

```
 you / routines ──launch──▶ Claude Code cloud sessions (1 per lane or task; 4 vCPU, 16 GB, no GPU)
                               │   secrets: DOPPLER_TOKEN → SessionStart hook → env
                               ├──▶ Modal GPU containers (one per experiment, per-second billing)
                               └──▶ shared ledger (Modal Dict) + git branches on GitHub
```

Claude Code cloud is the agent runtime. It's good at running many long-lived sessions, but its VMs
have no GPU, so heavy compute goes to Modal. Doppler holds the secrets. Each cloud environment
holds exactly one secret, a read-only Doppler service token scoped to an agents-only config.

## One-time setup

### 1. GitHub
Push this repo to GitHub and install the Claude GitHub App on it (claude.ai/code prompts you).
Cloud sessions clone from GitHub and push their `ar/...` branches back.

### 2. Modal (GPU compute)
```bash
uv run modal setup                      # browser login
infra/secrets/bootstrap_modal.sh        # creates the holdout-salt secret + cache volume
```
Then, in the Modal dashboard, create an API token for agents and **set a workspace spend limit**.
Fifty lanes running H100s overnight costs real money.

### 3. Doppler (secrets)
elbows lives in its own Doppler workspace, separate from any other Doppler account you use. Log in
with a token scoped to the repo directory, so `doppler` inside `elbows/` uses that workspace and
everywhere else keeps your default login:
```bash
doppler login --scope "$(git rev-parse --show-toplevel)" --yes   # sign in with the elbows account
doppler me                                                        # inside the repo: shows the elbows workspace
doppler setup -p elbows -c dev --no-interactive                   # local dev: `doppler run -- <cmd>`
```
Pass `--scope` an absolute path; a typo there makes the CLI silently fall back to the global login.

```bash
infra/secrets/bootstrap_doppler.sh      # project `elbows`, config `dev_agents`, service token
doppler secrets set -p elbows -c dev MODAL_TOKEN_ID MODAL_TOKEN_SECRET   # shared infra → root `dev`
doppler secrets set -p elbows -c dev_agents AR_LEDGER=modal              # agent-only settings
```
Configs are organised by **infrastructure, not by who uses it**:

| config | holds | used by |
|---|---|---|
| `dev` (root) | shared dev infra credentials: Modal, later HF / W&B / storage | humans (`doppler run -- …`), manual training |
| `dev_agents` (branch of `dev`) | inherits all of `dev`, plus agent-only settings (`AR_*`) | Claude Code cloud sessions, via the `DOPPLER_TOKEN` service token |
| `stg` / `prd` | later: anything touching real robots or production | never agents |

`dev_agents` gets a secret when it's *written* to `dev`. If you delete a secret in `dev_agents`, it
stays deleted there until it's next written in `dev`. Because everything in `dev` flows to agents,
and agents (or a prompt injection) can read their whole environment, keep anything agents
shouldn't have out of `dev` (`dev_personal`, or `stg`/`prd`).

### 4. Claude Code cloud environment
At claude.ai/code, open the environment selector → **Add environment**, name it `elbows-agents`:

| field | value |
|---|---|
| Network access | **Custom**: paste `infra/cloud/allowed_domains.txt`, tick "include default list" |
| Setup script | paste `infra/cloud/setup.sh` (installs Doppler CLI + Modal; needs no secrets) |
| Environment variables | `DOPPLER_TOKEN=dp.st.dev_agents.…` (only this) |
| Network secrets | optional, see below |

Then run `/remote-env` in the CLI and make `elbows-agents` the default.

### 5. Repo settings (needs your approval)
Copy `infra/cloud/claude-settings.proposed.json` to `.claude/settings.json`. It wires the
SessionStart hook that loads Doppler secrets, raises the Bash timeout to 15 min (experiments are
long-running foreground commands), and pre-approves the `ar`/git commands the loop uses. Hooks in
your personal `~/.claude/settings.json` don't reach cloud sessions; this file does.

### How secrets flow
1. The setup script runs before env vars exist, so it only installs the `doppler` binary.
2. On session start, `infra/cloud/session_start.sh` runs `doppler secrets download` with
   `$DOPPLER_TOKEN` and appends `export KEY=…` lines to `$CLAUDE_ENV_FILE`. Claude Code sources that
   file before every Bash command. Values are never printed into the transcript.
3. Rotating a secret in Doppler takes effect in new sessions. Revoking the service token cuts off
   every environment at once.

**Why Doppler, and what else to consider.** Doppler is a good choice: service tokens per config,
audit logs, one-click rotation, and syncs to GitHub Actions, Modal, AWS and others. **Infisical** is
the open-source near-equivalent if you want to self-host. **1Password service accounts** suit you
if the team already lives in 1Password. Claude Code also has **network secrets**: the proxy injects
a header for listed hosts, so the agent never sees the key. Use those for plain-HTTPS APIs (W&B,
Hugging Face, internal APIs) on top of Doppler. They can't carry Modal's credentials, which travel
over gRPC rather than a simple header, and they don't apply to setup scripts.

## Launching work

**Research lanes** (see `autoresearch/README.md`):
```bash
autoresearch/scripts/launch.sh <task> "a:focus…" "b:focus…" "c:focus…"
```

**General coding tasks:** one cloud session per independent task, from any terminal:
```bash
claude --cloud "Implement X per docs/specs/x.md; open a PR"
claude --cloud "Fix flaky test in …; open a PR"
claude --teleport            # pull a cloud session into your local terminal
```
Locally, `claude -w <name> --tmux` gives each session its own git worktree.

**Recurring work:** `/schedule` creates routines (cron, GitHub-event, or API-triggered cloud
sessions). Suggested first routine: daily at 07:07, run `autoresearch/prompts/review.md` (the
research director) on all tasks.

## Known limits (as of Nov 2026, check the docs as they change)
- Cloud VMs: about 4 vCPU / 16 GB / 30 GB disk, no GPU. Org-level cap on concurrent sessions
  (undocumented number). Usage counts against your Claude plan's rate limits.
- Setup script: < 5 min, cached ~7 days, no env vars.
- Idle sessions pause and lose background processes. This is why `ar run` is a blocking
  foreground command, not a background job.
- No public API for creating ad-hoc sessions other than `claude --cloud` and routine `/fire`
  triggers. For >20 lanes, or GPU-colocated agents, use self-hosted environments
  (`claude --cloud --environment ccpool_…`) or run Claude Code headless inside your own containers.
