<script lang="ts">
  import { fade } from 'svelte/transition';
  import { invoke } from '@tauri-apps/api/core';
  import { popouts } from './popouts.svelte';
  import { injectIntoActiveTerminal } from './terminalInject';
  import { llmModels } from './llmModels.svelte';

  interface Props {
    onclose: () => void;
    /** Open a new session tab (App.svelte owns the session manager). */
    onNewTab: () => void;
    /** Detach / dock the GUI cockpit (App.svelte owns the window state). */
    onToggleCockpit: () => void;
  }

  let { onclose, onNewTab, onToggleCockpit }: Props = $props();

  /** A slash-command discovered on disk under ~/.claude (backend scan). */
  interface DiscoveredCommand {
    name: string;
    source: 'command' | 'skill' | 'plugin' | string;
    description: string | null;
  }

  type Category = 'freq' | 'app' | 'llm' | 'claude' | 'run';

  interface LauncherEntry {
    id: string;
    label: string;
    sub?: string;
    icon: string;
    category: Category;
    /** Optional right-aligned tag (e.g. "claude"). Survives freq-wrapping. */
    badge?: string;
    action: () => void;
  }

  let query = $state('');
  let selectedIdx = $state(0);
  let inputEl: HTMLInputElement = $state(undefined!);
  let panelEl: HTMLDivElement = $state(undefined!);
  let claudeCommands = $state<DiscoveredCommand[]>([]);

  // ---------------------------------------------------------------------------
  // Draggable position (persisted to localStorage, matching Popout's size
  // persistence). `null` = use the default centered placement from CSS; once
  // dragged, the panel switches to fixed positioning at the saved coords.
  // ---------------------------------------------------------------------------
  const POS_KEY = 'rift:slash-launcher-pos';

  /** Clamp (x, y) so the panel stays on-screen; keeps at least the drag handle
   *  reachable even if the viewport shrank since the position was saved. */
  function clampToViewport(x: number, y: number): { x: number; y: number } {
    const w = panelEl?.offsetWidth ?? 560;
    const h = panelEl?.offsetHeight ?? 200;
    const maxX = Math.max(0, window.innerWidth - w);
    const maxY = Math.max(0, window.innerHeight - Math.min(h, 96));
    return {
      x: Math.min(Math.max(0, x), maxX),
      y: Math.min(Math.max(0, y), maxY),
    };
  }

  function readSavedPos(): { x: number; y: number } | null {
    try {
      const raw = localStorage.getItem(POS_KEY);
      if (!raw) return null;
      const p = JSON.parse(raw);
      if (typeof p?.x !== 'number' || typeof p?.y !== 'number') return null;
      return clampToViewport(p.x, p.y);
    } catch {
      return null;
    }
  }

  let pos = $state<{ x: number; y: number } | null>(readSavedPos());
  let dragging = $state(false);
  let dragOffX = 0;
  let dragOffY = 0;

  function startDrag(e: PointerEvent) {
    if (!panelEl) return;
    const rect = panelEl.getBoundingClientRect();
    dragOffX = e.clientX - rect.left;
    dragOffY = e.clientY - rect.top;
    dragging = true;
    try {
      (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    } catch {
      /* capture is best-effort; drag still works via the handle's events */
    }
    e.preventDefault();
  }

  function onDragMove(e: PointerEvent) {
    if (!dragging) return;
    pos = clampToViewport(e.clientX - dragOffX, e.clientY - dragOffY);
  }

  function endDrag(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
    } catch {
      /* no-op */
    }
    if (pos) {
      try {
        localStorage.setItem(POS_KEY, JSON.stringify(pos));
      } catch {
        /* localStorage unavailable — position just won't persist */
      }
    }
    inputEl?.focus();
  }

  /** Double-click the handle to reset to the default centered placement. */
  function resetPos() {
    pos = null;
    try {
      localStorage.removeItem(POS_KEY);
    } catch {
      /* ignore */
    }
    inputEl?.focus();
  }

  // Type a command into the active terminal (bracketed paste — never executes;
  // the user reviews + presses Enter). The injector appends a trailing space.
  function typeIntoTerminal(text: string): void {
    const ok = injectIntoActiveTerminal(text);
    if (!ok) {
      // No terminal in this window (e.g. detached cockpit) — surface rather
      // than silently drop the gesture.
      popouts.summon({
        content: { kind: 'text', title: 'No active terminal', body: `Couldn't send "${text}" — no terminal is focused in this window.` },
      });
    }
    onclose();
  }

  // Discover ~/.claude slash-commands once when the launcher opens. The scan is
  // disk-only (filenames + frontmatter); selecting one types it into the
  // terminal where the foreground agent (e.g. Claude Code) receives it.
  $effect(() => {
    void (async () => {
      try {
        claudeCommands = await invoke<DiscoveredCommand[]>('list_slash_commands');
      } catch {
        claudeCommands = [];
      }
    })();
  });

  const appEntries = $derived<LauncherEntry[]>([
    { id: 'app:settings', label: 'Settings', icon: '⚙', category: 'app',
      action: () => { popouts.summon({ content: { kind: 'settings' } }); onclose(); } },
    { id: 'app:new', label: 'New tab', icon: '⊕', category: 'app',
      action: () => { onNewTab(); onclose(); } },
    { id: 'app:project', label: 'Switch project', icon: '▦', category: 'app',
      action: () => { popouts.summon({ content: { kind: 'project-picker' } }); onclose(); } },
    { id: 'app:cockpit', label: 'Toggle cockpit (detach / dock)', icon: '⊞', category: 'app',
      action: () => { onToggleCockpit(); onclose(); } },
  ]);

  // LLM ops — quick access to models + prompt surfaces. Gated on the Ensemble
  // Router being enabled (otherwise there are no models to reach). Mirrors the
  // model logic in CommandPalette so behavior stays consistent.
  const llmEntries = $derived.by((): LauncherEntry[] => {
    if (!llmModels.enabled) return [];
    const items: LauncherEntry[] = [
      { id: 'llm:router', label: 'Router prompt', sub: 'Send a prompt to the active model', icon: '◆', category: 'llm',
        action: () => { popouts.summon({ content: { kind: 'llm-chat' }, width: 'min(720px, 85vw)' }); onclose(); } },
      { id: 'llm:ensemble', label: 'Ensemble compare', sub: 'Run one prompt across two models', icon: '⊞', category: 'llm',
        action: () => { popouts.summon({ content: { kind: 'llm-ensemble' }, width: 'min(1100px, 95vw)' }); onclose(); } },
    ];
    for (const m of llmModels.availableModels) {
      const isActive = llmModels.activeModelId === m.id;
      const isLocal = m.hosting.mode === 'local';
      const status = llmModels.processStatus[m.id];
      const live = status === 'running' || status === 'starting';
      const name = `${m.short_id || '?'} ${m.display_name || m.model_identifier}`;
      const icon = status === 'running' ? '●' : status === 'starting' ? '◐' : status === 'error' ? '✕' : '○';
      // A stopped local model needs its server started; activateModel hot-swaps
      // (stops other local servers to free VRAM, then starts this one). Cloud or
      // already-live models just point the router here.
      const verb = isLocal && !live ? 'Start & activate' : 'Activate';
      items.push({
        id: `llm:model:${m.id}`,
        label: `${verb}: ${name}${isActive ? '  (active)' : ''}`,
        icon,
        category: 'llm',
        action: () => { void llmModels.activateModel(m.id); onclose(); },
      });
      if (isLocal && live) {
        items.push({
          id: `llm:stop:${m.id}`,
          label: `Stop: ${name}`,
          icon: '■',
          category: 'llm',
          action: () => { void llmModels.stopModel(m.id); onclose(); },
        });
      }
    }
    return items;
  });

  const claudeEntries = $derived<LauncherEntry[]>(
    claudeCommands.map((c) => ({
      id: `claude:${c.source}:${c.name}`,
      label: `/${c.name}`,
      sub: c.description ?? undefined,
      icon: c.source === 'skill' ? '✦' : c.source === 'plugin' ? '⧉' : '»',
      category: 'claude',
      badge: 'claude',
      // No trailing space — the injector adds one (pasteTextIntoTerminal).
      action: () => typeIntoTerminal(`/${c.name}`),
    })),
  );

  // ---------------------------------------------------------------------------
  // Frequently-used section. Per-entry run counts persist to localStorage; when
  // the input is empty, the top few most-used commands surface at the top so the
  // things you reach for are one keystroke away. Run-in-terminal entries are not
  // counted (their text is dynamic, so a count would be meaningless).
  // ---------------------------------------------------------------------------
  const FREQ_KEY = 'rift:slash-launcher-freq';

  function loadFreq(): Record<string, number> {
    try {
      return JSON.parse(localStorage.getItem(FREQ_KEY) || '{}') || {};
    } catch {
      return {};
    }
  }

  // Read once per open — running an entry closes the launcher, so live updates
  // aren't needed; the next open reflects the new counts.
  const freqCounts = loadFreq();

  function bumpFreq(id: string): void {
    if (!id || id.startsWith('run:')) return;
    try {
      const c = loadFreq();
      c[id] = (c[id] || 0) + 1;
      localStorage.setItem(FREQ_KEY, JSON.stringify(c));
    } catch {
      /* localStorage unavailable — frequency just won't persist */
    }
  }

  /** Count the run against the underlying entry id (strip any freq: wrapper),
   *  then perform the action. */
  function runEntry(entry: LauncherEntry | undefined): void {
    if (!entry) return;
    bumpFreq(entry.id.replace(/^freq:/, ''));
    entry.action();
  }

  // Normalized query — a leading slash is the launcher's sigil, not part of the
  // search term, so `/aeg` matches the `aegis` command.
  const q = $derived(query.trim().replace(/^\//, '').toLowerCase());

  const filtered = $derived.by((): LauncherEntry[] => {
    const all = [...appEntries, ...llmEntries, ...claudeEntries];
    const matched = q
      ? all.filter((e) => e.label.toLowerCase().includes(q) || (e.sub?.toLowerCase().includes(q) ?? false))
      : all;
    // Frequently-used: while browsing (empty query) the top-used commands surface
    // at the top. Wrapped with a freq: id so the {#each} key stays unique vs the
    // same entry shown again in its own section below.
    const freq: LauncherEntry[] = q
      ? []
      : all
          .filter((e) => (freqCounts[e.id] || 0) > 0)
          .sort((a, b) => (freqCounts[b.id] || 0) - (freqCounts[a.id] || 0))
          .slice(0, 5)
          .map((e) => ({ ...e, id: `freq:${e.id}`, category: 'freq' as Category }));
    // Run-in-terminal: whatever the user typed (incl. a leading slash) sent
    // verbatim to the active terminal. Only offered when there's text to run.
    const run: LauncherEntry[] = query.trim()
      ? [{
          id: 'run:shell',
          label: `Run in terminal: ${query.trim()}`,
          icon: '▶',
          category: 'run',
          action: () => typeIntoTerminal(query.trim()),
        }]
      : [];
    return [...freq, ...matched, ...run];
  });

  $effect(() => {
    void filtered.length;
    selectedIdx = 0;
  });

  $effect(() => {
    if (inputEl) inputEl.focus();
  });

  function trapFocus(e: KeyboardEvent) {
    if (e.key !== 'Tab' || !panelEl) return;
    const focusable = panelEl.querySelectorAll<HTMLElement>(
      'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function onKeydown(e: KeyboardEvent) {
    trapFocus(e);
    if (e.key === 'Escape') {
      e.preventDefault();
      onclose();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      selectedIdx = Math.min(selectedIdx + 1, filtered.length - 1);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      selectedIdx = Math.max(selectedIdx - 1, 0);
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      runEntry(filtered[selectedIdx]);
      return;
    }
  }

  function categoryLabel(cat: Category): string {
    if (cat === 'freq') return 'FREQUENT';
    if (cat === 'app') return 'RIFT';
    if (cat === 'llm') return 'LLM';
    if (cat === 'claude') return 'CLAUDE COMMANDS';
    if (cat === 'run') return 'TERMINAL';
    return String(cat).toUpperCase();
  }
</script>

<div class="palette-backdrop" role="presentation" onclick={onclose} onkeydown={onKeydown} transition:fade={{ duration: 150 }}>
  <div
    class="palette-panel"
    class:positioned={pos !== null}
    style={pos ? `left: ${pos.x}px; top: ${pos.y}px;` : ''}
    role="dialog"
    aria-label="Slash launcher"
    aria-modal="true"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={onKeydown}
    bind:this={panelEl}
  >
    <!-- Drag handle — move the launcher; double-click to recenter. Position
         persists to localStorage. Decorative grip; keyboard users don't drag. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="drag-handle"
      class:dragging
      onpointerdown={startDrag}
      onpointermove={onDragMove}
      onpointerup={endDrag}
      ondblclick={resetPos}
      title="Drag to move · double-click to recenter"
    >
      <span class="grip" aria-hidden="true">⠿</span>
    </div>
    <input
      bind:this={inputEl}
      bind:value={query}
      onkeydown={onKeydown}
      placeholder="/ run a command — Rift actions, LLM models, Claude commands, or the terminal…"
      role="combobox"
      aria-label="Slash command launcher"
      aria-expanded={filtered.length > 0}
      aria-controls="slash-listbox"
      aria-activedescendant={filtered.length > 0 ? `slash-option-${selectedIdx}` : undefined}
      aria-autocomplete="list"
      spellcheck="false"
      autocomplete="off"
    />
    <div class="results" id="slash-listbox" role="listbox" aria-label="Slash commands">
      {#each filtered as entry, i (entry.id)}
        {@const showHeader = i === 0 || filtered[i - 1]?.category !== entry.category}
        {#if showHeader}
          <div class="category-header" role="presentation">{categoryLabel(entry.category)}</div>
        {/if}
        <div
          id="slash-option-{i}"
          class="entry"
          class:selected={i === selectedIdx}
          role="option"
          tabindex="-1"
          aria-selected={i === selectedIdx}
          onclick={() => runEntry(entry)}
          onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); runEntry(entry); } }}
          onmouseenter={() => { selectedIdx = i; }}
        >
          <span class="entry-icon">{entry.icon}</span>
          <span class="entry-text">
            <span class="entry-label">{entry.label}</span>
            {#if entry.sub}<span class="entry-sub">{entry.sub}</span>{/if}
          </span>
          {#if entry.badge}
            <span class="entry-badge">{entry.badge}</span>
          {/if}
        </div>
      {/each}
      {#if filtered.length === 0}
        <div class="empty" role="status">No matches</div>
      {/if}
    </div>
  </div>
</div>

<style>
  .palette-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: var(--backdrop-overlay);
    display: flex;
    justify-content: center;
    padding-top: 14vh;
  }

  .palette-panel {
    width: min(560px, 88vw);
    max-height: 460px;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  /* Once dragged, the panel is pinned at saved viewport coords (out of the
     backdrop's flex flow). Inline left/top supply the position. */
  .palette-panel.positioned {
    position: fixed;
    margin: 0;
  }

  .drag-handle {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 16px;
    flex-shrink: 0;
    cursor: grab;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-elevated);
    touch-action: none;
  }
  .drag-handle.dragging {
    cursor: grabbing;
  }
  .grip {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    line-height: 1;
    letter-spacing: 0.1em;
    user-select: none;
  }
  .drag-handle:hover .grip {
    color: var(--amber-warm);
  }

  input {
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--border-subtle);
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-lg);
    padding: var(--space-12) var(--space-lg);
    outline: 2px solid transparent;
  }
  input::placeholder {
    color: var(--amber-faint);
    opacity: 0.6;
  }
  input:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  .results {
    overflow-y: auto;
    padding: var(--space-xs) 0;
  }

  .category-header {
    padding: var(--space-8) var(--space-lg) var(--space-xs);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    color: var(--amber-faint);
    text-transform: uppercase;
  }

  .entry {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-lg);
    cursor: pointer;
    font-size: var(--text-md);
    color: var(--term-white);
  }
  .entry.selected {
    background: rgba(255, 200, 64, 0.10);
  }
  .entry-icon {
    flex-shrink: 0;
    width: 1.2em;
    text-align: center;
    color: var(--amber-warm);
  }
  .entry-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .entry-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .entry-sub {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .entry-badge {
    flex-shrink: 0;
    font-size: var(--text-2xs);
    color: var(--blue-claude, var(--amber-faint));
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    padding: 0 var(--space-xs);
    letter-spacing: 0.06em;
  }
  .empty {
    padding: var(--space-lg);
    text-align: center;
    color: var(--amber-faint);
    font-size: var(--text-md);
  }
</style>
