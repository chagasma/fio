---
id: tui-actions
title: Preserve navigation and act on questions and debts
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

Keep the user's place when navigating the TUI and let them act on demands, questions and debts without leaving it.

## Current state

Implemented and covered by tests: Esc restores selection, tab, scroll, filters and query; refresh keeps the exact row; `s` and `P` open status and priority pickers; `a` answers a question, resolves a debt or records a decision, question or debt. Pending: interactive terminal validation.

### Remaining acceptance checks

Reconstructed on 2026-10-14 from the implementation; interactive checks remain pending.

- In an isolated workspace, filter a list, select a row, open a detail tab, scroll and return with Esc. Verify selection, tab, scroll, filters and search state are restored at the appropriate navigation level.
- Change files externally and refresh with F5; the same row identity remains selected when present, and removing it leaves a valid selection.
- Change status and priority with their pickers, then cancel another change and verify no mutation occurs.
- Answer a specific question and resolve a specific debt from the relevant views; verify only the intended item changes and the open-item list updates.
- Create a decision, question and debt through the action picker; verify persisted content and correct demand ownership.
- Check usable keyboard navigation, help and error feedback in both narrow and wide terminals.

## Decisions

- DEC1 (2026-10-13): Replace the free-text status prompt with a picker so invalid values cannot be entered.
- DEC2 (2026-10-13): Rows carry a stable RowId so selection survives refresh and back navigation.



## Questions

## Debts
