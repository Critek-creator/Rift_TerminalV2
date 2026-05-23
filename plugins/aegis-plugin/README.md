# Aegis Plugin for Claude Code

Abyssal Aegis is a command center for Claude Code sessions. It provides
structured multi-mode task analysis: brainstorm, plan, critical review,
build with constraints, audit, research, and maintenance automation.

## Install

```bash
# From the Rift Terminal repo:
claude plugins install --source directory plugins/aegis-plugin

# Or add to your settings.json:
# "extraKnownMarketplaces": {
#   "aegis": { "source": { "source": "directory", "path": "/path/to/plugins/aegis-plugin" } }
# }
```

## Usage

```
/aegis              # auto-detect mode from task
/aegis --plan       # design architecture
/aegis --research   # deep investigation
/aegis --maintain   # check overdue tasks
/aegis --help       # full mode list
```

## What's included

- **Skill**: Aegis command center (SKILL.md + starter modes)
- **Hooks**: Maintenance banner on SessionStart, session context prompt
- **Scripts**: Maintenance state tracking (check, update, banner)
- **Commands**: `/aegis-help` quick reference card

## Rift Terminal Integration

When Rift Terminal is running, Aegis publishes bus envelopes that light up
the Aegis notification tab in the cockpit. Enable via Settings > Integrations.

## License

MIT — Abyssal Arts
