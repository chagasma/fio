# fio

A demand-based context manager for development with agents. Demands, decisions, questions, debts and session summaries live in Markdown files under `.fio/` in each workspace. People, agents and scripts can use the CLI; running `fio` opens the terminal interface.

## Install from source

With Rust 1.99 or later, run this in the cloned repository:

```sh
cargo install --path . --locked
```

Cargo installs the executable in `~/.cargo/bin/fio` (or your configured Cargo directory). Add that directory to PATH to use fio in any workspace, independently of this source checkout. Prebuilt binaries and automated releases are still pending.

## Use in a project

```sh
cd /path/to/workspace
fio init
fio new po-flow "Implement the purchase order flow"
fio use po-flow
fio context
fio ask po-flow "Can an order have multiple suppliers?"
fio decision po-flow "Validate purchase orders in the backend"
fio priority po-flow high
fio debt po-flow "Remove the quotation mock after integration"
fio search "purchase order"
fio search "supplier" --json
fio
```

On Linux, the project registry is `~/.config/fio/config.toml`, respecting `XDG_CONFIG_HOME`. Demand records are `.fio/demands/<id>/demand.md`, and sessions are Markdown files in its `sessions/` directory. The active demand is stored in `.fio/state.toml`; ignore that file and `.fio/` lock files in Git.

Search uses a disposable SQLite FTS5 index at `~/.cache/fio/index.db`, respecting `XDG_CACHE_HOME`. It is rebuilt when opening the TUI, running `fio search`, pressing F5, or receiving a file change notification. Removing the index does not remove any records. Search matches word prefixes across demand and session Markdown, including linked document paths; it does not read linked document contents or agent transcripts.

## Terminal interface

Projects → demands → details, with Summary, Sessions, Decisions, Questions, Debts and Docs tabs. Global views show active/blocked demands, open questions and open debts across projects. The interface supports demand actions and watches registered `.fio/` directories for changes. Live reload preserves the current demand and tab. An asterisk beside a demand ID marks the workspace’s selected default; this is independent of its status.

- Enter opens the selection; Esc goes back.
- Arrow keys or j/k move; PageUp/PageDown and Home/End navigate longer views.
- 1–5 switch global views outside demand details; 1–6 switch detail tabs. Tab/Shift+Tab also switch tabs.
- `f` cycles the status filter; `p` cycles priority; `x` clears filters. These filters apply to demand and open-item lists; search covers all projects.
- `/` enters search; `:` enters a command; F5 refreshes while preserving navigation.
- `n` creates a demand in the selected project; `s` prompts for its status; `u` selects it as the workspace default. Enter submits and Esc cancels.
- `c` copies the selected demand’s context to the system clipboard. Clipboard availability depends on the desktop session.
- `e` opens the demand in `VISUAL` or `EDITOR`. In Sessions or Docs, choose an entry and press Enter to open it. Quoted editor arguments are supported.
- `r` lists sessions with recorded native IDs; Enter runs `codex resume <id>` or `claude --resume <id>` in the demand’s workspace. fio returns when the process exits.
- `?` shows help and any unreadable project/demand warnings; `q` or Ctrl+C exits.

Commands: `projects`, `demands`, `active`, `questions`, `debts`, `search <text>`, `refresh`, `help`, `quit`.

## Agents and parallel sessions

```sh
fio session start --demand po-flow --agent codex --objective "Implement the backend" --json
fio session start --demand po-flow --agent claude --objective "Implement the portal" --json
fio session list --json
```

Each start returns a distinct local ID. Each agent keeps its own ID and ends only its own record:

```sh
fio session end <local-session-id> --summary "Work completed and verified" --next "Remaining work"
```

`--state "current state"` also replaces the demand's current-state summary. Read the demand again before replacing it when other agents are working on it. Writes are serialized to preserve concurrent item updates. `fio use` changes only the default for commands without an explicit demand; it does not change open sessions.

Structured reads support `--json`: `projects`, `list`, `show`, `context`, `search` and `session list`. Shapes differ by purpose: `show --json` keeps the frontmatter as written (relative `docs`, `related` as ID strings) and lists sessions with their local `id`; `context --json` is for agents (absolute `docs`, relations as `{id,title,status,path,note}`); `session list --json` returns the local `id` plus the native `session_id` and `transcript`. Unreadable records are reported as warnings on stderr and skipped by `list`, `search` and session commands. To integrate another project, add the demand-context section from `AGENTS.md` to its Codex instructions, and copy `.claude/skills/fio/` into the same location in that project. The skill can also live in `~/.claude/skills/fio/` for personal use across projects.

See `fio --help` and `fio session --help`. Session records do not start or stop agent processes. Project-local lifecycle hooks can link native session IDs and transcript paths automatically. New templates and generated output use English; user-authored content stays in its original language.

## Automatic agent integration

Install hooks separately in each fio workspace:

```sh
fio hooks install claude
fio hooks install codex
```

The installer merges fio handlers into `.claude/settings.local.json` or `.codex/hooks.json`, preserving other settings and hooks. Reinstalling replaces only fio handlers. It records the absolute path of the executable used for installation; rerun the installer after moving the executable. During source development, use `cargo run -- hooks install <agent>`; for other workspaces, install fio with Cargo first. Local hook settings should stay out of Git because they contain machine-specific paths.

Restart the agent after installation. Codex requires reviewing and trusting the project hooks through `/hooks`; fio does not change trust settings. Hooks are supported in the locally verified Codex 0.162.1 and Claude Code 2.1.296. See the official [Codex hooks](https://learn.chatgpt.com/docs/hooks) and [Claude Code hooks](https://code.claude.com/docs/en/hooks) references for current requirements.

SessionStart opens or reuses a fio record using the native session ID, links the transcript path, and injects demand context and the local fio ID. New native sessions use the workspace's active demand. For a project dedicated to one demand, `fio hooks install codex --demand <id>` pins new bindings explicitly. Once bound, a native session stays with its original demand even if another agent changes the shared default. Resume reuses an open record or opens a new record for that same demand after closure. Compaction reuses the record. Hooks skip subagent events and workspaces without `.fio/`; without an active demand, they ask the agent to obtain the user's selection.

Stop saves the last assistant response in a separate quoted section. It requests an authored session summary at most once per turn when it has not been updated. Agents use `fio session update <local-id> --summary "Work completed and verified" --next "Remaining work"` without closing the session. SessionEnd closes only the matching native session's record. If no work summary was written, it explicitly records that completion was not verified. Hooks never automatically replace the demand's Current state or extract decisions from prose. Storage errors are reported as hook warnings and allow the agent to continue.

Transcripts stay at their original paths; fio does not read or copy them. No API calls or transcript scans are required. Forced termination may prevent SessionEnd from running; use `fio session list` and close the relevant local record explicitly if needed.
