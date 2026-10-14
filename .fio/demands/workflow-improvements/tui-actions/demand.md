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

## Decisions

- DEC1 (2026-10-13): Replace the free-text status prompt with a picker so invalid values cannot be entered.
- DEC2 (2026-10-13): Rows carry a stable RowId so selection survives refresh and back navigation.



## Questions

## Debts
