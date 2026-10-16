---
id: tui-context
title: Preview context and improve session resumption
project: fio
status: "active"
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

Preview the demand context inside the TUI and make session resumption easier to choose.

## Current state

Implemented and covered by tests: `v` opens a scrollable full-context preview, the list shows a responsive preview panel, and the resume picker shows objective, summary and next steps per session. Pending: interactive terminal validation.

### Remaining acceptance checks

Reconstructed on 2026-10-14 from the implementation; interactive checks remain pending.

- Open full context with `v`, scroll through long content, close it and confirm the underlying selection is preserved.
- Compare the full preview with CLI context for the same explicit demand; objective, state, open items and recent sessions should agree.
- Resize between narrow and wide terminals; the preview must not make the demand list unusable.
- Open the resume picker with multiple native session IDs; verify objective, summary and next steps let the user distinguish choices, and cancellation launches nothing.
- Resume a deliberately selected test session in its demand workspace and verify fio restores terminal interaction after the external process exits.
- Confirm missing native session IDs and missing handoff notes produce understandable feedback. Parallel-session aggregation and malformed-record tolerance remain scoped to shared-context and workspace-health.

## Decisions

## Questions

## Debts
