# fio development

Follow the scope and stages in `fio-plano.md`. Before starting a new stage, present a short plan and wait for user approval. Do not add tests, extra documentation, logs or compatibility shims unless requested. The integration instructions below are part of Stage 3. Use English for code, comments, interfaces, generated templates and project documentation. Preserve user-authored demand content.

# Demand context

In workspaces with `.fio/`, use the fio CLI to maintain demand context. In this repository, substitute `cargo run --` for `fio` if the executable is not on PATH. In other projects, use the installed executable.

- At the start of work, use `fio context <injected-demand-id>` if a hook supplied a binding. Otherwise, run `fio context` to read the active demand. If the user specifies a demand, select it with `fio use <id>` and read its context. If none is active, check `fio list`; do not select or create arbitrary demands.
- If a SessionStart hook supplies a local fio session ID, reuse that record. Do not open a duplicate. Before finishing each turn, run `fio session update <local-session-id> --summary "Work completed and verified" --next "Next steps and blockers"`. Use `--state` only after rereading the demand. The SessionEnd hook closes the record; do not close it at the end of each response.
- Without an injected hook record, open your record with `fio session start --demand <id> --agent codex --objective "Session objective" --json` and keep the returned local `id`. This differs from the native Codex session ID. When available, supply real values for `--session-id` and `--transcript`. Parallel sessions are allowed, including within the same demand. Do not end another agent's sessions.
- Record decisions with `fio decision <id> "Decision and reason"`, questions with `fio ask <id> "Question"` and debts with `fio debt <id> "Pending work and resolution condition"`.
- Record confirmed answers with `fio answer <id> Q<n> "Answer"` and resolve completed debts with `fio resolve <id> D<n>`.
- Link existing files with `fio link <id> <paths...>`, relative to the current directory. Do not move documents into `.fio/`.
- When finishing or interrupting a manually opened session, run `fio session end <local-session-id> --summary "Work completed and verified" --next "Next steps and blockers" --state "Current demand state"`. Describe incomplete work as incomplete. State explicitly when there are no next steps.

`fio context --json`, `fio show <id> --json`, `fio list --json`, `fio search "term" --json` and `fio projects --json` provide structured reads. `fio session list --json` lists open sessions; `--demand <id>` filters by demand. The selection made by `fio use` is a shared default: parallel agents should use explicit IDs in `context`, `session start` and `session end`. Changing the active demand does not affect open sessions. Writes to the same demand are serialized; `--state` replaces Current state, so read the demand again and preserve other agents' progress when writing that summary. SessionStart/Stop/SessionEnd hooks can link native session IDs and transcript paths automatically. Hook output identifies the exact local record to update. Transcripts remain references; do not scan unrelated agent conversations. The CLI records context without controlling Codex processes.
