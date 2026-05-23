---
description: Create an Abyssal Index scaffold at ~/.claude/abyssal-index/
---

Create the Abyssal Index directory structure for first-time users.

Check if `~/.claude/abyssal-index/MAIN_INDEX.md` already exists:
- If yes: report "Index already exists at ~/.claude/abyssal-index/" and stop.
- If no: create the scaffold.

## Scaffold structure

Create the following directories and files:

```
~/.claude/abyssal-index/
  MAIN_INDEX.md
  indexes/
    projects.md
    research.md
    skills.md
    practices.md
  vaults/
    pr001.md
```

## MAIN_INDEX.md content

```markdown
# MAIN_INDEX

AIX MAIN INDEX | updated: <today> | format: telegraphic-english

INDEXES:
  projects   -> indexes/projects.md   | #vaults: 0
  research   -> indexes/research.md   | #vaults: 0
  skills     -> indexes/skills.md     | #vaults: 0
  practices  -> indexes/practices.md  | #vaults: 1

QUICK-REF:
  global-rules = pr001

!RULES:
  1. Scan this index first -- always
  2. Identify relevant indexes from task context
  3. Load category index -> find vault IDs
  4. Load only relevant vaults (max 5 per session unless urgent)
  5. After session -> update vaults with new knowledge
```

## Category index files

Each index file gets a header with the category name and empty template.

## pr001.md (starter practices vault)

```markdown
VAULT: pr001 | Global Practices | updated: <today>

--- CORE RULES ---
  Add your project-wide rules and conventions here.
  This vault is loaded on every session start.

--- LESSONS ---
  Add gotchas, failure patterns, and fixes here.
  Format: what happened, the fix, when it applies.
```

After creating, report the scaffold structure and suggest running
`/index-query` to verify.
