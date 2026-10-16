---
id: shared-context
title: Improve shared context and concurrent state updates
project: fio
status: backlog
priority: medium
parent: workflow-improvements
created: "2026-10-13"
updated: "2026-10-13"
links: {}
related: []
blocked_by: []
docs: []
---

## Objective

Make handoffs between concurrent agent sessions explicit and prevent stale demand-state updates from silently replacing newer progress. Preserve each session's contribution and provide enough context to resume the right work.

## Current state

Backlog. Recovered theme: the existing title. The concrete design below was proposed on 2026-10-14 after inspecting src/context.rs, src/store/sessions.rs and src/store/demands.rs.

Per-demand and workspace locks serialize writes, but session `--state` replaces Current state without comparing the state previously read by the caller. Two individually valid writes can therefore lose semantic progress. Context currently selects the latest three session records by start time; an older open parallel session can be omitted. The side preview takes next steps only from the latest-started session, even when its next steps are empty.

### Proposed behavior

1. Expose a state revision token in structured context and show reads. Derive it from the actual Current state content or otherwise detect manual edits, not only CLI writes.
2. Introduce a guarded state-update operation requiring the revision the caller read. Under the existing lock, compare revisions and reject stale updates with the current state and actionable conflict information. Exact CLI spelling is an implementation choice.
3. Route session `--state` through the same conflict protection. Keep session summary/next-step updates independent of shared-state replacement. Specify the outcome clearly if a combined request conflicts; proposed behavior is to reject before any session/state mutation so retrying is unambiguous.
4. Show all open sessions in a dedicated context section and limit recent closed sessions separately. Include local record ID, agent, objective, summary and next steps, so the recipient can identify whose progress is being described.
5. Make previews disclose parallel sessions and missing handoff notes instead of representing one latest-started record as the complete next step. Keep user-authored Current state authoritative; do not automatically merge prose or infer completion from the latest assistant response.

### Example

Agents A and B read revision R1. A records a new state, creating R2. B's update based on R1 fails with a conflict and leaves A's state intact. B rereads R2, incorporates both contributions, and retries. Both session records remain independently identifiable.

### Acceptance criteria

- The stale-write scenario above cannot silently overwrite newer progress, including after a manual Markdown state edit.
- A rejected combined session/state update leaves both records unchanged and explains how to retry.
- Updating only a session summary does not replace demand Current state.
- More than three concurrent open sessions remain visible in context; recent closed-session limits do not hide open work.
- Text and JSON agree on session identity, lifecycle and missing notes. Closed records never appear as open.
- Changing the shared active demand does not retarget an already-bound session.
- File locking and atomic writes remain in place; no agent process control or transcript scanning is introduced.

## Decisions

## Questions

## Debts
