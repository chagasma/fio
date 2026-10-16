---
id: workspace-health
title: Diagnose workspace issues and tolerate invalid session records
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

Diagnose invalid or inconsistent workspace records and keep valid demands and sessions usable when an individual record is broken. Give the user a file path, explanation and concrete repair guidance.

## Current state

Backlog. Recovered theme: the existing title. The following specification was proposed on 2026-10-14 from current code.

Demand catalog scans and workspace session scans already collect warnings. However, src/store/sessions.rs::read fails the entire demand's session read when any record is invalid. This strict path is used by src/context.rs, CLI show, TUI session details, previews and resume choices. The search index reads raw session Markdown independently, so a searchable record is not proof of valid session metadata. No dedicated diagnostic command exists.

### Proposed behavior

1. Add a read-only `fio doctor` command, with `--json` and explicit current-workspace/all-projects scope. Report a stable issue code, severity, project, demand when known, path, explanation and suggested repair.
2. Check workspace configuration and active-demand references; demand frontmatter, IDs and parent/folder consistency; session frontmatter and ownership; missing linked documents; missing related/blocked_by targets; and dependency cycles. Treat missing resources as diagnostics, not deletion instructions.
3. Share tolerant session loading across context, show and TUI read paths. Return valid records plus warnings; surface warnings in human output and structured output instead of silently treating a partial result as complete.
4. Add a visible health/warning entry in the TUI with inspectable issue details and an editor action for an existing source file. Keep unrelated valid data navigable. Refresh must clear repaired issues.
5. Keep explicit writes strict: an unreadable selected record cannot be overwritten merely because read views tolerate it. Do not auto-repair, delete records or close open sessions.

### Example

One session file contains invalid YAML while two others are valid. The demand context, session list and resume choices still expose the valid records; the damaged path and parse failure are visible. Correcting that file and refreshing removes the issue.

### Acceptance criteria

- A malformed session cannot prevent inspecting the valid portion of a demand or unrelated projects.
- CLI text, JSON and TUI expose the same underlying issues with source paths; machine-readable diagnostics do not depend on parsing stderr prose.
- A healthy workspace produces an empty issue list. Proposed command exit codes: 0 for no issues, 1 for reported issues, 2 when diagnostics cannot run.
- Missing default demand, dangling dependency, wrong session ownership and missing linked document are distinguishable findings.
- Running diagnostics leaves workspace records unchanged. Existing user-authored text is preserved during any later manual repair.
- Repaired records become usable after refresh without restarting fio.

## Decisions

## Questions

## Debts
