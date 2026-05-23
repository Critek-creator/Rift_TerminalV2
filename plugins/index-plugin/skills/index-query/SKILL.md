---
description: "Query the Abyssal Index — search vaults, browse domains, read entries. Use when the user asks 'what do I know about X', 'pull my thoughts on Y', 'search my index', 'what's in my vaults about Z'."
---

# Index Query

Search and browse the Abyssal Index at `~/.claude/abyssal-index/`.

## How to query

1. Read `~/.claude/abyssal-index/MAIN_INDEX.md` for the catalog
2. Identify the relevant category index (projects, research, skills, etc.)
3. Read the category index to find vault IDs
4. Read the relevant vault files directly

## Vault format

Vaults are markdown files with a YAML-style header line:
```
VAULT: <id> | <title> | updated: <date>
```

Content uses telegraphic English — compressed natural language. Read natively.

## Categories

| Category | Index file | Content |
|----------|-----------|---------|
| Projects | indexes/projects.md | Project status, stack, repo paths |
| Research | indexes/research.md | Tech stack knowledge, gotchas |
| Skills | indexes/skills.md | Skill design rationale |
| Practices | indexes/practices.md | Global rules, lessons, hygiene |
| Lore | indexes/lore.md | Creative/worldbuilding content |
| Agents | indexes/agents.md | Agent personas and deployment |
