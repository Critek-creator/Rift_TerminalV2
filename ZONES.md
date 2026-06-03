# Rift Fixed Zone Model (N2)

*Status: ACTIVE · Phase 2 of the IA north-star (`RIFT_IA_NORTHSTAR.md` §N2) · 2026-06-03*
*Source of truth: `src/lib/zones.ts` · enforced by `src/lib/__tests__/zones.test.ts`*

## Why this exists

North-star principle **N2 — "the layout IS the IA."** Every category of content
gets exactly **one** home, and a new feature must *claim a zone* — it cannot
float as an orphan. Before this model the zones lived only implicitly in
`App.svelte`'s markup; nothing declared them, which is how "data everywhere"
took hold. `zones.ts` is now the single source of truth and `zones.test.ts`
fails CI if a zone goes empty, a surface is registered twice, or a surface
claims a non-existent zone.

## The six zones

| Zone | Persistence | Role |
|---|---|---|
| **chrome** | fixed | Window furniture: identity, window controls, tab navigation. |
| **terminal** | fixed | The shell surface — the primary work area. One or more PTY panes. |
| **cockpit** | toggle | Right-hand observability column: the Index graph and the file tree. |
| **notification** | toggle | Categorized activity surfaces — the notif-tab system + its promoted side-pane. |
| **status** | fixed | Bottom ambient chrome: context keybind hints + the live status line. |
| **overlay** | transient | Surfaces that float above everything and dismiss on Esc/backdrop. |

`fixed` = always present · `toggle` = user can collapse/detach · `transient` =
appears on demand, then dismisses.

## Surface → zone assignments

Every top-level surface claims exactly one zone (`src/lib/zones.ts` `SURFACES`):

| Surface | Zone | Rendered by |
|---|---|---|
| Title bar | chrome | `TitleBar.svelte` |
| Tab bar | chrome | `TabBar.svelte` |
| Update banner | chrome | `App.svelte` (inline) |
| Terminal grid | terminal | `TerminalGrid.svelte` |
| Empty state | terminal | `App.svelte` (inline) |
| Index graph | cockpit | `IndexGraph.svelte` |
| File tree | cockpit | `Tree.svelte` |
| Promoted notif pane | notification | `NotificationPane.svelte` + `*TabContent.svelte` |
| Mode-hint bar | status | `ModeHintBar.svelte` |
| Status line | status | `StatusLine.svelte` |
| Command palette | overlay | `CommandPalette.svelte` |
| Shortcut overlay | overlay | `ShortcutOverlay.svelte` |
| Welcome overlay | overlay | `WelcomeOverlay.svelte` |
| Pop-out stack | overlay | `Popout.svelte` |
| Close-tab confirm | overlay | `App.svelte` (inline) |

### Notification tabs

The individual notif tabs are not listed surface-by-surface — they are
registered dynamically in `sectionCatalog.svelte.ts` and **all live in the
`notification` zone by rule** (`NOTIF_ZONE`). The notif-tab *system* is
represented above by the single `promoted-pane` surface. Each tab additionally
claims one of three groups (`system` / `activity` / `intel`) for the collapsed
tab dropdowns.

## Orphan resolution

The IA audit flagged two tabs as "undocumented orphans." Both are legitimate
first-party surfaces and are **kept**, now explicitly accounted for:

| Tab | Resolution |
|---|---|
| `integrations` ("links") | Integration Capability Inspector. Kept · zone `notification` · group `system`. |
| `feature-pipeline` ("pipeline") | Feature Pipeline (idea-store scan). Kept · zone `notification` · group `intel`. |

`zones.test.ts` carries a regression guard asserting both stay registered and
grouped, so they cannot silently drift back into orphan status.

## Adding a surface

1. Add a `SurfaceDescriptor` to `SURFACES` in `src/lib/zones.ts` with its `zone`.
2. If it's a notif tab, register it in `sectionCatalog.svelte.ts` instead — it
   inherits the `notification` zone automatically.
3. `npx vitest run zones` must stay green.
