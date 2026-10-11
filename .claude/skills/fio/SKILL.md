---
name: fio
description: Resume demand context and record decisions, questions, debts and session summaries with the fio CLI in workspaces containing .fio/.
---

Use the CLI from the demand's workspace or a subdirectory. In this repository, substitute `cargo run --` for `fio` if it is not on PATH. In other projects, use the installed executable; do not run another project's Cargo to execute fio.

At the start of work, use `fio context <injected-demand-id>` when a hook supplied a binding. Otherwise, read `fio context` for the active demand. If the user specifies another demand, select it with `fio use <id>` and read its context. If none is active, check `fio list`; do not choose an arbitrary demand or create one just to record the conversation.

If SessionStart injected a local fio session ID, reuse that record instead of opening another. Before finishing each turn, run `fio session update <local-session-id> --summary "Work completed and verified" --next "Next steps and blockers"`; update `--state` only after rereading the demand. SessionEnd closes hook-linked records automatically. Do not close them at the end of each response.

Without an injected hook record, open a record with `fio session start --demand <id> --agent claude --objective "Session objective" --json`. Keep the returned local `id`; it identifies your fio record and differs from Claude's native session ID. When the native ID and transcript path are available, supply `--session-id` and `--transcript`; never invent these values. Parallel sessions are allowed, including in the same demand. Preserve other agents' sessions.

During work, record relevant events as they happen:

- `fio decision <id> "Decision and reason"`
- `fio ask <id> "Unanswered question"`
- `fio answer <id> Q<n> "Confirmed answer"`
- `fio debt <id> "Technical debt and resolution condition"`
- `fio resolve <id> D<n>` once the debt is resolved.

Link existing documents with `fio link <id> <paths...>`. Paths are relative to the current directory; files stay where they are. Read only documents needed for the task. Structured reads are available through `fio show <id> --json`, `fio context <id> --json` and `fio search "term" --json`.

When completing or interrupting a manually opened session, run:

```sh
fio session end <local-session-id> --summary "Work completed and verified" --next "Next steps and blockers" --state "Current demand state"
```

Summaries and state must reflect what happened, including incomplete work. State explicitly when there are no next steps. `fio session list --json` lists open sessions, optionally filtered by `--demand <id>`. The active demand is a shared default: during parallel work, use explicit IDs in `fio context <id>`, `session start --demand <id>` and `session end <local-id>`. Changing the active demand does not alter existing sessions. Writes are serialized to preserve parallel agents' items. Before using `--state`, read the demand again and preserve other agents' progress: this option replaces Current state. These commands record context; they do not start, resume or stop Claude Code processes. Installed SessionStart/Stop/SessionEnd hooks link native IDs and transcript paths automatically. Use the injected local ID for updates. Transcripts remain references; do not scan unrelated conversations. Use English for generated content unless the user requests another language; preserve existing user-authored content.
