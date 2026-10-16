---
id: attention-view
title: Surface questions, blockers and open sessions
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

Provide one actionable overview of unanswered questions, unresolved debts, blocked demands and open session records, across registered projects or within a selected project. Make the reason for attention visible before the user opens a demand.

## Current state

Backlog. Recovered theme: the existing title. The scope below was proposed on 2026-10-14 after source inspection; it is not a recovered transcript.

The TUI already has separate Active, Questions and Debts screens and actions for answering/resolving items. It has no combined attention screen or global session screen. src/tui/rows.rs builds these lists, src/tui/input.rs selects screens, and src/store/sessions.rs exposes open records. Reuse these capabilities instead of duplicating item mutation logic.

### Proposed behavior

1. Add an Attention view reachable by a named command and visible help entry. Show project scope, counts by reason, and active filters.
2. Show separate, identifiable rows for unanswered questions, unresolved debts, blocked demands and open session records. Each row includes project, demand, reason and an appropriate short summary. Session rows include agent, start time and objective.
3. Separate actionable blockers/questions from informational open-session activity. Use a deterministic order within groups: demand priority, then stable project/demand/item identity. Do not present every open session as an error.
4. Enter opens the relevant demand tab or session details. Reuse the existing answer/resolve actions and resume picker where applicable. Preserve scope, filters and selection on return and reload.
5. Display blocked_by references when available; a blocked demand without references remains visible with an explicit missing-detail message. Missing referenced demands remain visible as unavailable.
6. Show time since a session started as recorded age, not as agent liveness. Do not close sessions automatically or invent a stale-session threshold in the first version.

### Example

A backend demand has Q2 awaiting a user answer and a frontend demand is blocked by it. The overview exposes both reasons. Answering Q2 removes that question row after refresh; it does not automatically change either demand's status. The user can inspect the dependency and decide whether to unblock the frontend.

### Acceptance criteria

- A mixed workspace exposes all four row types with unambiguous project and demand identity, including duplicate demand IDs in different projects.
- Answered questions, resolved debts and closed sessions leave their corresponding groups after reload; a blocked demand remains until its status changes.
- Answering or resolving acts on the selected item, and cancelling leaves it unchanged.
- Returning from details and receiving file changes preserve the selected identity when it still exists; removing it chooses a valid neighboring row.
- Empty and partially unreadable workspaces show useful empty states and visible warnings without hiding valid results.
- Narrow terminals retain the reason and demand identity; counts reflect the displayed scope and filters.

## Decisions

## Questions

## Debts
