// sessionManager.svelte.ts — A-01 extraction step 1
//
// Owns session tab state, split/close pane logic, dead-session tracking,
// project-per-tab derivation, and the project_swap debounce effect.

import { invoke } from '@tauri-apps/api/core';
import { replaceLeaf, removeLeaf, collectLeafIds } from './splitTypes';
import type { SplitNode } from './splitTypes';
import type { SessionTab, ActiveSurface } from './TabBar.svelte';
import { popouts } from './popouts.svelte';
import {
  getRestorePayload,
  clearSnapshot,
  gatherPanes,
  writeSnapshot,
  type RestorePayload,
} from './sessionRestore';

// ---------------------------------------------------------------------------
// Stage 2b restart-safe boot restore
// ---------------------------------------------------------------------------

/** Highest leaf (pane) id anywhere in a layout tree — used to seed the id
 *  counter above every restored id so later splits never collide. */
function maxLeafId(node: SplitNode): number {
  return node.type === 'terminal'
    ? node.id
    : Math.max(maxLeafId(node.children[0]), maxLeafId(node.children[1]));
}

/** Build the initial session list from a restore payload, reusing the ORIGINAL
 *  pane ids so each pane re-hydrates its own buffer by id. `null` when there is
 *  nothing to restore (panes mount fresh as the default single session). */
function buildRestoredSessions(payload: RestorePayload | null): SessionTab[] | null {
  if (!payload) return null;
  const layout: SplitNode | null = payload.layout
    ? (payload.layout as SplitNode)
    : payload.panes.length > 0
      ? { type: 'terminal', id: payload.panes[0].pane_id }
      : null;
  if (!layout) return null;
  const leaves = collectLeafIds(layout);
  if (leaves.length === 0) return null;
  const proj = payload.panes[0]?.project_root ?? null;
  // Tab id == its first (original) leaf id, matching how a fresh tab's id and
  // its single pane id coincide (one monotonic namespace).
  return [{ id: leaves[0], title: 'rift', projectPath: proj, layout }];
}

// Top-level await: a single fast file read so panes mount already in the
// restored layout (no throwaway default shell to spawn-then-kill). Falls back
// to the default single session when restore is disabled or nothing qualifies.
const bootPayload = await getRestorePayload();
const restoredSessions = buildRestoredSessions(bootPayload);
if (restoredSessions && bootPayload) {
  // The in-memory payload stays cached for per-pane hydration; clearing only
  // deletes the on-disk file so it does not replay on the next boot.
  clearSnapshot(bootPayload.session_id);
}

const DEFAULT_SESSIONS: SessionTab[] = [
  { id: 0, title: 'rift', projectPath: null, layout: { type: 'terminal', id: 0 } },
];

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let nextSessionId = restoredSessions
  ? maxLeafId(restoredSessions[0].layout) + 1
  : 1;
let initialProjectRoot = $state<string | null>(null);
let sessions = $state<SessionTab[]>(restoredSessions ?? DEFAULT_SESSIONS);
const firstLeaf = collectLeafIds((restoredSessions ?? DEFAULT_SESSIONS)[0].layout)[0] ?? 0;
let focusedSessionId = $state(firstLeaf);
let active = $state<ActiveSurface>({ kind: 'session', id: sessions[0].id });

const paneSessionPaths = new Map<number, string | null>();

// One-shot commands to type into a pane once its PTY is live, keyed by pane
// (leaf) id. Used by features that open a fresh terminal to run a command —
// e.g. the Settings → Gemini "sign in" button launches `gemini` here so the
// user lands in the interactive OAuth flow without typing anything.
const pendingInitialCommands = new Map<number, string>();

let exitedPaneIds = $state(new Set<number>());
let pendingCloseId = $state<number | null>(null);

// ---------------------------------------------------------------------------
// Derived
// ---------------------------------------------------------------------------

const deadSessions: Set<number> = $derived.by(() => {
  const dead = new Set<number>();
  for (const s of sessions) {
    const leafIds = collectLeafIds(s.layout);
    if (leafIds.length > 0 && leafIds.every((id) => exitedPaneIds.has(id))) {
      dead.add(s.id);
    }
  }
  return dead;
});

const activeProjectPath = $derived.by(() => {
  const a = active;
  if (a.kind !== 'session') return initialProjectRoot;
  const panePath = paneSessionPaths.get(focusedSessionId);
  if (panePath) return panePath;
  const s = sessions.find((s) => s.id === a.id);
  return s?.projectPath ?? initialProjectRoot;
});

const multiProject = $derived(
  new Set(sessions.map((s) => s.projectPath)).size > 1,
);

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

function activateSession(id: number) {
  active = { kind: 'session', id };
}

/** Open a fresh terminal tab and queue `command` to run in it once the PTY is
 *  live. Returns the new pane id, or `undefined` if a tab could not be opened.
 *  The Terminal component consumes the queued command after `pty_start`. */
function openTerminalWithCommand(command: string): number | undefined {
  const id = addSession();
  if (id !== undefined) pendingInitialCommands.set(id, command);
  return id;
}

/** Consume (read-and-clear) the one-shot command queued for a pane, if any.
 *  Called once by Terminal after its PTY starts. */
function consumeInitialCommand(paneId: number): string | undefined {
  const cmd = pendingInitialCommands.get(paneId);
  if (cmd !== undefined) pendingInitialCommands.delete(paneId);
  return cmd;
}

function addSession(opts?: { pickProject?: boolean }): number | undefined {
  if (opts?.pickProject) {
    openProjectPickerForNewTab();
    return undefined;
  }
  const id = nextSessionId++;
  const activeSession = sessions.find(
    (s) => active.kind === 'session' && s.id === active.id,
  );
  const inheritedPath = activeSession?.projectPath ?? initialProjectRoot;
  const projectName = inheritedPath?.replace(/\\/g, '/').split('/').pop() ?? '';
  const defaultTitle = projectName || `terminal ${sessions.length + 1}`;
  sessions = [
    ...sessions,
    { id, title: defaultTitle, projectPath: inheritedPath, layout: { type: 'terminal', id } },
  ];
  active = { kind: 'session', id };
  focusedSessionId = id;
  return id;
}

function handleSplit(terminalId: number, direction: 'hsplit' | 'vsplit'): void {
  const newId = nextSessionId++;
  const activeSession = sessions.find(
    (s) => active.kind === 'session' && s.id === active.id,
  );
  if (!activeSession) return;

  const inheritedPath = activeSession.projectPath ?? initialProjectRoot;
  const newLeaf: SplitNode = { type: 'terminal', id: newId };
  const splitNode: SplitNode = {
    type: direction,
    children: [{ type: 'terminal', id: terminalId }, newLeaf],
    ratio: 0.5,
  };

  sessions = sessions.map((s) => {
    if (s.id !== (active.kind === 'session' ? active.id : -1)) return s;
    return { ...s, layout: replaceLeaf(s.layout, terminalId, splitNode) };
  });

  const parentTabId = active.kind === 'session' ? active.id : -1;
  const parentTab = sessions.find((s) => s.id === parentTabId);
  if (parentTab) {
    paneSessionPaths.set(newId, inheritedPath);
  }

  focusedSessionId = newId;
}

function handleClosePane(terminalId: number): void {
  const activeId = active.kind === 'session' ? active.id : -1;
  const targetSession = sessions.find((s) => s.id === activeId);
  if (!targetSession) return;

  const newLayout = removeLeaf(targetSession.layout, terminalId);
  if (newLayout === null) {
    closeSession(activeId);
    return;
  }

  sessions = sessions.map((s) =>
    s.id === activeId ? { ...s, layout: newLayout } : s,
  );

  paneSessionPaths.delete(terminalId);

  if (focusedSessionId === terminalId) {
    const remaining = collectLeafIds(newLayout);
    if (remaining.length > 0) focusedSessionId = remaining[0];
  }
}

function openProjectInNewTab(path: string) {
  const id = nextSessionId++;
  const projectName =
    path.replace(/\\/g, '/').split('/').pop() ?? `terminal ${sessions.length + 1}`;
  sessions = [
    ...sessions,
    { id, title: projectName, projectPath: path, layout: { type: 'terminal', id } },
  ];
  active = { kind: 'session', id };
  focusedSessionId = id;
}

function openProjectPickerForNewTab() {
  popouts.summon({
    content: {
      kind: 'project-picker',
      onSelect: openProjectInNewTab,
    },
  });
}

function reorderSession(srcId: number, dstId: number) {
  const srcIdx = sessions.findIndex((s) => s.id === srcId);
  const dstIdx = sessions.findIndex((s) => s.id === dstId);
  if (srcIdx < 0 || dstIdx < 0 || srcIdx === dstIdx) return;
  const next = [...sessions];
  const [moved] = next.splice(srcIdx, 1);
  next.splice(dstIdx, 0, moved);
  sessions = next;
}

function renameSession(id: number, title: string) {
  sessions = sessions.map((s) => (s.id === id ? { ...s, title } : s));
}

function markPaneExited(paneId: number) {
  exitedPaneIds.add(paneId);
}

/** Stage 2b: snapshot the entire active session — every leaf pane's buffer/cwd
 *  plus the tiling layout tree — in one write. Driven by the focused pane's
 *  timer + onDestroy (see Terminal.svelte). Skips a *partial* capture (a pane
 *  provider mid-teardown) so a complete snapshot is never clobbered by a
 *  half-gone one during app close. */
function captureActiveSession() {
  if (active.kind !== 'session') return;
  const activeId = active.id;
  const s = sessions.find((x) => x.id === activeId);
  if (!s) return;
  const leafIds = collectLeafIds(s.layout);
  const panes = gatherPanes(leafIds);
  if (panes.length === 0 || panes.length !== leafIds.length) return;
  writeSnapshot(panes, s.layout);
}

function cleanupSessionResources(id: number) {
  const session = sessions.find((s) => s.id === id);
  if (session) {
    const leafIds = collectLeafIds(session.layout);
    for (const lid of leafIds) {
      exitedPaneIds.delete(lid);
      paneSessionPaths.delete(lid);
      pendingInitialCommands.delete(lid);
    }
    exitedPaneIds = exitedPaneIds;
  }
}

function closeSession(id: number) {
  const session = sessions.find((s) => s.id === id);
  if (session) {
    const leafIds = collectLeafIds(session.layout);
    const hasAlive = leafIds.some((lid) => !exitedPaneIds.has(lid));
    if (hasAlive && pendingCloseId !== id) {
      pendingCloseId = id;
      return;
    }
  }
  pendingCloseId = null;
  cleanupSessionResources(id);
  sessions = sessions.filter((s) => s.id !== id);
  if (active.kind === 'session' && active.id === id) {
    const last = sessions.at(-1);
    active = last ? { kind: 'session', id: last.id } : { kind: 'empty' };
  }
}

function confirmClose() {
  if (pendingCloseId !== null) {
    const id = pendingCloseId;
    pendingCloseId = null;
    cleanupSessionResources(id);
    sessions = sessions.filter((s) => s.id !== id);
    if (active.kind === 'session' && active.id === id) {
      const last = sessions.at(-1);
      active = last ? { kind: 'session', id: last.id } : { kind: 'empty' };
    }
  }
}

function cancelClose() {
  pendingCloseId = null;
}

// ---------------------------------------------------------------------------
// Project swap debounce effect
// ---------------------------------------------------------------------------

let lastSwappedPath: string | null = null;
let swapTimer: ReturnType<typeof setTimeout> | undefined;

$effect.root(() => {
  $effect(() => {
    const path = activeProjectPath;
    if (!path || path === lastSwappedPath) return;
    lastSwappedPath = path;
    clearTimeout(swapTimer);
    swapTimer = setTimeout(() => {
      invoke('project_swap', { path }).catch((err: unknown) =>
        console.warn('[rift] tab-switch project_swap failed:', err),
      );
    }, 100);
  });

  return () => {
    clearTimeout(swapTimer);
  };
});

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

export const sessionManager = {
  get sessions() { return sessions; },
  set sessions(v: SessionTab[]) { sessions = v; },
  get focusedSessionId() { return focusedSessionId; },
  set focusedSessionId(v: number) { focusedSessionId = v; },
  get active() { return active; },
  set active(v: ActiveSurface) { active = v; },
  get initialProjectRoot() { return initialProjectRoot; },
  set initialProjectRoot(v: string | null) { initialProjectRoot = v; },
  get deadSessions() { return deadSessions; },
  get activeProjectPath() { return activeProjectPath; },
  get multiProject() { return multiProject; },
  get pendingCloseId() { return pendingCloseId; },
  get exitedPaneIds() { return exitedPaneIds; },
  get paneSessionPaths() { return paneSessionPaths; },
  activateSession,
  addSession,
  openTerminalWithCommand,
  consumeInitialCommand,
  handleSplit,
  handleClosePane,
  openProjectInNewTab,
  openProjectPickerForNewTab,
  reorderSession,
  renameSession,
  markPaneExited,
  captureActiveSession,
  closeSession,
  confirmClose,
  cancelClose,
};
