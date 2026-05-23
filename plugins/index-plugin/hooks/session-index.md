---
event: SessionStart
type: prompt
---

The Abyssal Index is available at `~/.claude/abyssal-index/`. It is a
hierarchical compressed memory system organized into vaults (markdown files
with YAML frontmatter) across category indexes.

On session start, scan the index for context relevant to the current task:
1. Read `~/.claude/abyssal-index/MAIN_INDEX.md` for the vault catalog
2. Identify relevant vaults from the task domain
3. Load relevant vaults for context

Use `/index-query` to search the index. Use `/index-setup` to create a
new index scaffold if one doesn't exist.
