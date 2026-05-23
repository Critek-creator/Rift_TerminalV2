# Abyssal Index Plugin for Claude Code

The Abyssal Index is a personal knowledge cockpit — a hierarchical compressed
memory system built from markdown vault files. It gives Claude Code persistent
context about your projects, tech stack, practices, and lessons learned.

## Install

```bash
# From the Rift Terminal repo:
claude plugins install --source directory plugins/index-plugin
```

## Usage

```
/index-setup        # create a new Index scaffold (first-time)
/index-query        # search and browse your vaults
```

## What's included

- **Skill**: `index-query` — search vaults, browse domains, read entries
- **Command**: `/index-setup` — create the scaffold at `~/.claude/abyssal-index/`
- **Hook**: Session context prompt that loads relevant vaults on start

## How it works

The Index stores knowledge in **vaults** — markdown files organized by category
(projects, research, skills, practices, lore). Each vault uses telegraphic
English for maximum information density at minimal token cost.

Categories are tracked in **index files** that map vault IDs to descriptions.
The **MAIN_INDEX.md** is the root catalog.

```
~/.claude/abyssal-index/
  MAIN_INDEX.md           # root catalog
  indexes/                # category indexes
    projects.md
    research.md
    skills.md
    practices.md
  vaults/                 # vault content
    pr001.md              # global practices
    p001.md               # project vault
    r001.md               # research vault
```

## Rift Terminal Integration

When Rift Terminal is running with Index integration enabled, the vault
browser tab in the cockpit shows your index content with search and
cross-referencing. Enable via Settings > Integrations.

## License

MIT — Abyssal Arts
