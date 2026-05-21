# Getting Started with Rift Terminal

Rift is a terminal emulator and developer cockpit built by Abyssal Arts. It runs your shell like any terminal, but watches everything that happens — commands, file changes, hooks, agent activity — and organizes it into a real-time dashboard alongside your work.

## System Requirements

| Platform | Minimum |
|----------|---------|
| Windows  | Windows 10 (build 1809+) |
| Linux    | Ubuntu 20.04+ / any distro with WebKit2GTK 4.1 |
| macOS    | macOS 12 Monterey+ |

Rift is built with [Tauri](https://tauri.app) — a lightweight Rust + web framework. Installers are self-contained; no runtime dependencies.

## Installation

Download the latest release from the [GitHub Releases page](https://github.com/Critek-creator/Rift_TerminalV2/releases).

| Platform | File | Notes |
|----------|------|-------|
| Windows  | `Rift-x.x.x-setup.exe` | NSIS installer. Currently unsigned — Windows SmartScreen may show a warning. Click "More info" → "Run anyway". |
| Linux    | `Rift_x.x.x_amd64.AppImage` | Make executable: `chmod +x Rift_*.AppImage`, then run. |
| macOS    | `Rift_x.x.x.dmg` | Currently unsigned — right-click the app → "Open" to bypass Gatekeeper on first launch. |

## First Launch

When you open Rift for the first time, a **Welcome Guide** walks you through the interface. You can reopen it anytime from **Settings > About > Show Welcome Guide**.

What you'll see:

- **Tab bar** across the top — session tabs on the left (your terminal sessions), notification tabs on the right (your cockpit).
- **Terminal** in the main area — your shell, ready to use.
- **Status line** at the bottom — shows directory, git branch, model, session info, and more.

## Lane Colors

Rift color-codes terminal output by source. Each "lane" has a distinct color:

| Color | Source |
|-------|--------|
| Blue (`#6CB6FF`) | Claude voice |
| Purple (`#C58FFF`) | Agent output |
| Cyan (`#6FE0E0`) | Hook events |
| Amber (`#FFA826`) | Aegis |
| Green (`#4FE855`) | Success / OK |
| Red (`#FF4848`) | Errors / warnings |

Tags like `CLAUDE`, `HOOK`, `ERR` appear as small bordered labels marking the source at a glance.

## The Cockpit (Notification Tabs)

The tabs on the right side of the tab bar are the cockpit. Each monitors a different subsystem:

| Tab | Icon | What it shows |
|-----|------|---------------|
| Errors | ⚡ | Aggregated errors and warnings |
| Hooks | ⚓ | Claude Code hook activity |
| Commands | ⌘ | Command history with exit codes |
| Files | ⊞ | Filesystem activity tree with heatmap |
| Git | ⎇ | Repository state changes |
| Bus Tail | ⌁ | Raw event firehose (for debugging) |
| Sessions | ⏱ | Session history and replay |

**Integration tabs** appear automatically when their integration is detected:

| Tab | Icon | Requires |
|-----|------|----------|
| Aegis | ◉ | Aegis command center active |
| Agents | ◊ | Agent activity detected |
| Index | ◈ | Abyssal Index connected |
| MCP | ⬡ | MCP server connected |
| Sentinel | ⊘ | Sentinel monitoring active |

### Working with Tabs

- **Click** a notification tab to view it in the main pane.
- **Drag** a tab off the strip to promote it to a side pane alongside the terminal.
- **Drag** a promoted pane back onto the strip to demote it.
- **Right-click** a tab to show/hide it.
- Click the **⧉** button on hover to pop a tab out to its own window.
- Click **⋯** at the end of the strip to manage tab visibility.

## Connecting Claude Code

Rift includes an MCP server that Claude Code can connect to. This gives Claude read/write access to the Rift event bus, file system, git state, and more.

1. The MCP plugin is at `plugins/rift-mcp-plugin/` in the Rift install.
2. Add it to your Claude Code MCP configuration (`.mcp.json`).
3. When connected, you'll see the **MCP** tab light up in the cockpit.

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+?` | Show keyboard shortcuts overlay |
| `Ctrl+K` | Command palette |
| `Ctrl+B` | Toggle cockpit panel |
| `Ctrl+Shift+F` | Search terminal |
| `Ctrl+=` / `Ctrl+-` | Zoom in / out |
| `Ctrl+0` | Reset zoom |
| `Ctrl+Shift+C` | Copy selection |
| `Ctrl+Shift+V` | Paste |
| `Escape` | Close overlay / dismiss |

## Settings

Click the gear icon in the title bar or use the command palette to open Settings. Available sections:

- **About** — version info, welcome guide
- **Crash Logs** — view/copy recorded errors for bug reports
- **Updates** — check for new versions
- **Project** — switch project root
- **Filesystem** — configure ignore patterns and tree depth
- **Index** — vault sync and display settings
- **Notifications** — tab filter thresholds

## Known Limitations (Beta)

- **Unsigned installers** — Windows SmartScreen and macOS Gatekeeper will warn on first launch. This is expected for the beta.
- **StatusLine segments** — CTX%, SESSION USE%, and WEEK% show "—" until Claude Code ships usage-reporting hooks.
- **No mobile/tablet client** — Rift is desktop-only for now.

## Reporting Issues

Found a bug? Have a feature request?

- **GitHub Issues**: [github.com/Critek-creator/Rift_TerminalV2/issues](https://github.com/Critek-creator/Rift_TerminalV2/issues)
- Include your OS, Rift version (Settings > About), and steps to reproduce.
- If Rift crashed, copy the crash log from Settings > Crash Logs and paste it into the issue.

## Support Development

Rift is free and open-source. If you find it useful, consider supporting development on [Patreon](https://patreon.com/abyssalarts).

---

*Built by Abyssal Arts — Rust + Tauri + Svelte*
