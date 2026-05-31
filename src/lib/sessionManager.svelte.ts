// sessionManager.svelte.ts — A-01 extraction step 1
//
// Owns session tab state, split/close pane logic, dead-session tracking,
// project-per-tab derivation, and the project_swap debounce effect.

import { invoke } from '@tauri-apps/api/core';
import { replaceLeaf, removeLeaf, collectLeafIds } from './splitTypes';
import type { SplitNode } from './splitTypes';
import type { SessionTab, ActiveSurface } from './TabBar.svelte';
import { popouts } from './popouts.svelte';

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let nextSessionId = 1;
let initialProjectRoot = $state<string | null>(null);
let sessions = $state<SessionTab[]>([
  { id: 0, title: 'rift', projectPath: null, layout: { type: 'terminal', id: 0 } },
]);
let focusedSessionId = $state(0);
let active = $state<ActiveSurface>({ kind: 'session', id: 0 });

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
  closeSession,
  confirmClose,
  cancelClose,
};
