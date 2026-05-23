---
description: "Abyssal Aegis — Claude Code command center. Multi-mode task analysis: brainstorm, plan, critical review, build, audit, research, maintain. Use when the user invokes /aegis or says 'aegis', 'plan this', 'audit this', 'research this', 'brainstorm this'."
---

# Abyssal Aegis — Starter Kit

This is the distributable starter version of Aegis. It provides the core
command center workflow with the most commonly used modes.

## Invocation

`/aegis` with optional flags: `--think`, `--plan`, `--crit`, `--guard`,
`--audit`, `--research`, `--maintain`, `--wrap`, `--help`.

## Mode Detection

Analyze the task description and select the appropriate mode:

| Signal | Mode |
|--------|------|
| "fix", "bug", "typo" | BASIC (inline fix, no subagents) |
| "brainstorm", "ideas" | BRAINSTORM — explore approaches |
| "plan", "how should I" | PLAN — design architecture |
| "audit", "review" | AUDIT — post-task check |
| "research", "look into" | RESEARCH — deep investigation |
| "maintain", "overdue" | MAINTAIN — maintenance sweep |
| "wrap", "session end" | WRAP — session synthesis |

## Core Workflow

1. Read the task description
2. Detect the appropriate mode
3. For BASIC: handle inline (no subagents)
4. For complex modes: deploy parallel subagents via the Agent tool
5. Synthesize findings into structured output
6. Surface a concrete NEXT action when applicable

## Maintenance

Run `/aegis --maintain` to check for overdue maintenance tasks.
The maintenance banner fires on session start if tasks are overdue.

## Full Version

This starter kit covers the essential modes. The full Aegis system
(30+ modes, vault integration, telemetry, dispatch tiers, self-audit)
is available at https://github.com/Critek-creator/Rift_TerminalV2.
