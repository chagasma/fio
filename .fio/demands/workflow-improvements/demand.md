---
id: workflow-improvements
title: Improve agent workflows and terminal usability
project: fio
status: "active"
priority: medium
created: "2026-10-13"
updated: "2026-10-14"
links: {}
related: []
blocked_by: []
docs: []
---

## Objective

Make fio a terminal workspace for observing demand progress, finding what needs attention, inspecting context and taking the next action without losing the user's place. Use k9s as an interaction reference: scoped resource views, keyboard navigation, contextual actions and live updates, adapted to demands and recorded agent sessions.

## Current state

Roadmap reconstructed on 2026-10-14 from the five existing sub-demands, commit 4de11a4 and current source. The original backlog records contained titles but no objectives or implementation scope. Their themes are recovered evidence; the detailed behavior and acceptance criteria below are new proposals, not a verbatim recovery of yesterday's conversation.

Already present: project and demand navigation, global active/question/debt views, status and priority filters, full-text search, file watching, editor integration and native session resumption. Commit 119fa44 added navigation restoration, item actions, context previews and richer resume choices. The two active sub-demands still require interactive terminal validation.

Suggested sequence: validate tui-actions and tui-context; implement workspace-health so incomplete data remains usable; implement attention-view as the main usability improvement; then implement shared-context to make parallel handoffs reliable. This is a proposed order, not a change to demand priorities or statuses.

### Product workflow

Open fio, select a project or all projects, see blocked work and unanswered questions, inspect the relevant demand, answer or resolve an item, and return to the same filtered view. When resuming work, see each recorded session's objective and next steps before choosing the native session.

### Additional ideas proposed during this reconstruction

- Add a discoverable command picker with available actions, shortcuts and project scope; the current colon prompt only recognizes exact commands.
- Offer a local text filter for the current view separately from global full-text search; today `/` navigates to global search.
- Add demand ordering by priority and update date, with visible ordering and stable selection after reload.
- Make blocked_by and related demands navigable and editable inside the TUI so users can follow a dependency to its cause.

These are follow-up candidates, not recovered commitments or additional implementation demands. Keep the first delivery focused on the existing five sub-demands.

### Boundaries and reference

Keep local Markdown as the source of truth. A recorded open session is not evidence of a running or healthy agent process. Do not introduce process orchestration or automatic transcript analysis as part of this roadmap.

Interaction reference: https://k9scli.io/topics/commands/ (resource commands, scoped views, filtering, contextual actions and Esc navigation).

## Decisions

- DEC1 (2026-10-13): Implement the approved roadmap incrementally, starting with actionable TUI navigation and context preview. Preserve local Markdown storage and validate with existing and targeted regression tests plus an isolated interactive terminal.
- DEC2 (2026-10-14): Reconstruct the existing roadmap in place using demand titles, repository history and current source. Label detailed acceptance criteria and additional k9s-inspired ideas as proposals dated 2026-10-14 because the original backlog bodies were empty; preserve existing implementation states and user-authored content.



## Questions

## Debts
