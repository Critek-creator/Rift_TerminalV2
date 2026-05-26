<script lang="ts">
  // Aegis integration tab — §10.8 four-section notification anatomy.
  //
  // Data sources (decision C, Phase 7.2):
  //   c1  aegis.context    — startup snapshot: SKILL.md version, hook count/events,
  //                          lesson count. Rare; typically one per session.
  //   c2  aegis.invocation — live tail of ~/.claude/aegis.log. One per line.
  //
  // Phase 7.3: quick-action buttons (open lessons / open settings in OS editor).
  //
  // pr003 svelte5-async-cleanup-via-sync-shell-iife: the cleanup returned from
  // $effect must be SYNC. Async teardown (bus unsubscribe) is wrapped in IIFE.

  import { invoke } from '@tauri-apps/api/core';
  import { subscribe, type Envelope } from './bus';
  import AegisTabRenderer from './AegisTabRenderer.svelte';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { shouldShow, type SeverityLevel } from './notifFilter';

  interface Props {
    /** Minimum severity for events to render. Default: info. */
    severityThreshold?: SeverityLevel;
    /** Drag-back handle for promoted-pane mode (Phase 3.5a). */
    onDragBack?: () => void;
  }

  let { severityThreshold = 'info', onDragBack }: Props = $props();

  const RECENT_LOG_LIMIT = 100;
  const LIVE_ACTIVITY_WINDOW_MS = 4000;

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let entries = $state<Envelope[]>([]);
  let lastTickTs = $state<number>(Date.now());

  // Latest aegis.context payload — drives the status header.
  let contextPayload = $state<{
    skill_version?: string;
    skill_updated?: string;
    hook_count?: number;
    hook_events?: string[];
    lesson_count?: number | null;
  } | null>(null);

  // ---------------------------------------------------------------------------
  // Derived views (§10.8 sections)
  // ---------------------------------------------------------------------------

  // Section 2 — live activity strip: events within the trailing 4-second window.
  const liveEntries = $derived.by(() => {
    const cutoff = lastTickTs - LIVE_ACTIVITY_WINDOW_MS;
    return entries.filter((e) => e.ts >= cutoff);
  });

  // Section 3 — recent events log: last N, newest first.
  const recentEntries = $derived(entries.slice(-RECENT_LOG_LIMIT).reverse());

  // Counts for the status header and persistent panel.
  const totalCount = $derived(entries.length);
  const invocationCount = $derived(entries.filter((e) => e.kind === 'aegis.invocation').length);

  const lastSeenLabel = $derived.by(() => {
    if (entries.length === 0) return '—';
    const last = entries[entries.length - 1];
    const ageMs = Math.max(0, lastTickTs - last.ts);
    if (ageMs < 1000) return 'just now';
    if (ageMs < 60_000) return `${Math.floor(ageMs / 1000)}s ago`;
    if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
    return `${Math.floor(ageMs / 3_600_000)}h ago`;
  });

  // Section 1 — status header label.
  const statusLabel = $derived.by(() => {
    if (!contextPayload) return 'Aegis · waiting for context…';
    const ver = contextPayload.skill_version ?? '?';
    const updated = contextPayload.skill_updated ? ` · updated ${contextPayload.skill_updated}` : '';
    const hooks = contextPayload.hook_count ?? 0;
    return `Aegis · v${ver}${updated} · ${hooks} hook${hooks === 1 ? '' : 's'} active`;
  });

  // ---------------------------------------------------------------------------
  // Envelope handler
  // ---------------------------------------------------------------------------

  function handleEnvelope(env: Envelope) {
    // aegis.context always passes — it's a structural event, not log noise.
    if (env.kind !== 'aegis.context' && !shouldShow(env.kind, severityThreshold)) return;
    if (env.kind === 'aegis.context') {
      contextPayload = env.payload as typeof contextPayload;
    }
    entries = [...entries, env];
    if (entries.length > RECENT_LOG_LIMIT * 2) {
      entries = entries.slice(-RECENT_LOG_LIMIT);
    }
    lastTickTs = Date.now();
  }

  // ---------------------------------------------------------------------------
  // Bus subscription + tick timer (Svelte 5 runes)
  // pr003 svelte5-async-cleanup-via-sync-shell-iife
  // ---------------------------------------------------------------------------

  let connected = $state(false);
  let connectError = $state<string | null>(null);
  let tickTimer: ReturnType<typeof setInterval> | undefined;
  let unsubscribeFn: (() => Promise<void>) | undefined;

  $effect(() => {
    // Async setup inside IIFE — cleanup returned from $effect must be sync.
    void (async () => {
      try {
        unsubscribeFn = await subscribe({ category: 'aegis' }, handleEnvelope);
        connected = true;
      } catch (err) {
        console.error('[AegisTabContent] bus subscribe failed', err);
        connectError = err instanceof Error ? err.message : String(err);
      }
    })();

    tickTimer = setInterval(() => {
      lastTickTs = Date.now();
    }, 1000);

    // Sync cleanup.
    return () => {
      if (tickTimer) clearInterval(tickTimer);
      // Async teardown in IIFE (pr003 svelte5-async-cleanup-via-sync-shell-iife).
      void (async () => {
        await unsubscribeFn?.();
      })();
    };
  });

  // ---------------------------------------------------------------------------
  // Phase 7.3 — quick-action button state
  // ---------------------------------------------------------------------------

  let quickActionError = $state<string | null>(null);
  let quickActionTimer: ReturnType<typeof setTimeout> | undefined;

  function clearErrorAfterDelay() {
    if (quickActionTimer) clearTimeout(quickActionTimer);
    quickActionTimer = setTimeout(() => {
      quickActionError = null;
      quickActionTimer = undefined;
    }, 3000);
  }

  function openLessons() {
    void (async () => {
      try {
        await invoke('aegis_open_lessons');
      } catch (err) {
        quickActionError = err instanceof Error ? err.message : String(err);
        clearErrorAfterDelay();
      }
    })();
  }

  function openSettings() {
    void (async () => {
      try {
        await invoke('aegis_open_settings');
      } catch (err) {
        quickActionError = err instanceof Error ? err.message : String(err);
        clearErrorAfterDelay();
      }
    })();
  }

  // ---------------------------------------------------------------------------
  // Drag-handle (Phase 3.5a promoted-pane)
  // ---------------------------------------------------------------------------

  function onHandleDragStart(e: DragEvent) {
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Marker MIME — TabBar.onStripDrop filters by NOTIF_TAB_MIME presence
      // and rejects drags missing it (silent demote failure if omitted).
      e.dataTransfer.setData(NOTIF_TAB_MIME, '__promoted_pane__');
      e.dataTransfer.setData('text/plain', '__promoted_pane__');
    }
  }
</script>

<section class="pane" data-accent="amber">
  {#if onDragBack}
    <div
      class="drag-handle"
      role="button"
      tabindex="0"
      draggable={true}
      ondragstart={onHandleDragStart}
      title="drag back to tab strip to dock"
    >
      <span class="handle-glyph" style="color: var(--amber-warm); font-size: 14px">◈</span>
      <span class="handle-title">AEGIS</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  {#if connectError}
    <div class="connect-error">{connectError}</div>
  {:else if !connected}
    <div class="connecting-state">Connecting…</div>
  {:else}
  <!-- Section 1: Status header -->
  <header class="status">
    <span class="title"><span class="icon">◉</span>{statusLabel}</span>
    <span class="spacer"></span>
    <span class="state">
      {totalCount} event{totalCount === 1 ? '' : 's'} · last {lastSeenLabel}
    </span>
  </header>

  <!-- Section 2: Live activity strip -->
  <div class="strip">
    <span class="strip-label">LIVE</span>
    {#if liveEntries.length === 0}
      <span class="strip-empty">(no in-flight events)</span>
    {:else}
      <div class="strip-events">
        {#each liveEntries as e, i (e.ts + ':' + e.kind + ':' + i)}
          <span class="strip-event">{e.kind}</span>
        {/each}
      </div>
    {/if}
  </div>

  <!-- Section 3: Recent events log -->
  <div class="log">
    <div class="log-header">RECENT EVENTS</div>
    <div class="log-body" aria-live="polite">
      {#if recentEntries.length === 0}
        <div class="empty-state">
          <span class="empty-state-icon">◈</span>
          <span class="empty-state-text">Aegis is listening</span>
          <span class="empty-state-hint">session intelligence events will surface here</span>
        </div>
      {:else}
        {#each recentEntries as e, i (e.ts + ':' + e.kind + ':' + i)}
          <AegisTabRenderer
            entry={{
              ts: e.ts,
              category: e.category as string,
              kind: e.kind,
              payload: (e.payload as Record<string, unknown>) ?? {},
            }}
          />
        {/each}
      {/if}
    </div>
  </div>

  <!-- Section 4: Persistent state panel -->
  <footer class="state-panel">
    <div class="state-header">PERSISTENT STATE</div>
    <div class="state-body">
      <div class="row k-row">
        <span class="k">skill version</span>
        <span class="v">{contextPayload?.skill_version ?? '—'}</span>
      </div>
      <div class="row k-row">
        <span class="k">last updated</span>
        <span class="v">{contextPayload?.skill_updated ?? '—'}</span>
      </div>
      <div class="row k-row">
        <span class="k">hooks active</span>
        <span class="v">{contextPayload?.hook_count ?? '—'}</span>
      </div>
      <div class="row k-row">
        <span class="k">lessons</span>
        <span class="v">
          {#if contextPayload?.lesson_count != null}
            {contextPayload.lesson_count} lessons
          {:else}
            —
          {/if}
        </span>
      </div>
      <div class="row k-row">
        <span class="k">invocations</span>
        <span class="v">{invocationCount}</span>
      </div>
      <!-- §10.13: active enforcement modes — placeholder until rift-aegis
           emits enforcement_modes in its aegis.context snapshot payload.
           NEEDS-INFO: Rust-side rift-aegis needs to emit enforcement_modes
           in its snapshot payload for this segment to show real data. -->
      <div class="row k-row">
        <span class="k">enforcement</span>
        <span class="v enforcement-placeholder">&mdash;</span>
      </div>

      <!-- Rule source tags (hook_events) -->
      {#if contextPayload?.hook_events && contextPayload.hook_events.length > 0}
        <div class="rule-sources">
          <span class="rs-label">RULE SOURCES</span>
          <div class="rs-tags">
            {#each contextPayload.hook_events as event (event)}
              <span class="aegis-rule-source-tag">{event}</span>
            {/each}
          </div>
        </div>
      {/if}

      <!-- Skill path — faint italic meta lane -->
      {#if contextPayload}
        <div class="skill-path-row">
          <span class="skill-path-label">skill path</span>
          <span class="skill-path">~/.claude/skills/aegis/SKILL.md</span>
        </div>
      {/if}

      <!-- Phase 7.3: quick-action buttons -->
      <div class="quick-actions">
        <button type="button" class="qa-btn" onclick={openLessons}>Open Lessons</button>
        <button type="button" class="qa-btn" onclick={openSettings}>Open Settings</button>
      </div>
      {#if quickActionError}
        <div class="qa-error">{quickActionError}</div>
      {/if}
    </div>
  </footer>
  {/if}
</section>

<style>
  .connecting-state {
    color: var(--amber-faint);
    padding: 1rem var(--space-lg);
    font-style: italic;
    font-size: var(--type-body-size);
    letter-spacing: var(--type-body-spacing);
  }
  .connect-error {
    color: var(--term-red);
    padding: var(--space-8) var(--space-lg);
    font-style: italic;
    font-size: var(--type-body-size);
    letter-spacing: var(--type-body-spacing);
    opacity: 0.9;
    box-shadow: var(--sep-depth);
  }
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-base);
    color: var(--amber-primary);
    font-family: var(--font-family);
    font-size: var(--text-base);
    --accent: var(--amber-primary, #FFA826);
  }

  /* Phase 3.5a drag handle */
  .drag-handle {
    height: var(--control-sm);
    padding: 0 var(--space-12);
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
    display: flex;
    align-items: center;
    gap: var(--space-md);
    cursor: grab;
    user-select: none;
    color: var(--amber-warm);
    font-size: var(--type-label-size);
    letter-spacing: var(--type-label-spacing);
    font-weight: var(--type-label-weight);
    transition: background var(--duration-base) ease-out;
  }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle .handle-glyph {
    color: var(--accent);
    font-size: var(--text-base);
    text-shadow: var(--glow-amber-faint);
  }
  .drag-handle .handle-title {
    color: var(--accent);
    text-transform: uppercase;
  }
  .drag-handle .handle-hint {
    margin-left: auto;
    color: var(--amber-faint);
    font-style: italic;
    font-weight: 400;
    letter-spacing: 0.04em;
  }

  /* Section 1: Status header */
  .status {
    height: 36px;
    padding: 0 var(--space-lg);
    background: var(--bg-elevated);
    box-shadow: var(--sep-glow);
    display: flex;
    align-items: center;
    gap: var(--space-14);
    color: var(--amber-warm);
    flex-shrink: 0;
  }
  .status .title {
    font-size: var(--type-section-size);
    font-weight: var(--type-section-weight);
    letter-spacing: var(--type-section-spacing);
    color: var(--accent);
    text-shadow: var(--glow-amber-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status .icon { margin-right: var(--space-8); opacity: 0.85; font-size: var(--text-lg); }
  .status .spacer { flex: 1; }
  .status .state {
    font-size: var(--type-caption-size);
    font-weight: var(--type-caption-weight);
    letter-spacing: var(--type-caption-spacing);
    color: var(--amber-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* Section 2: Live activity strip */
  .strip {
    height: var(--control-sm);
    padding: 0 var(--space-lg);
    box-shadow: var(--sep-depth);
    display: flex;
    align-items: center;
    gap: var(--space-14);
    background: linear-gradient(to bottom, rgba(212, 137, 10, 0.05), transparent);
    color: var(--amber-dim);
    font-size: var(--type-label-size);
    letter-spacing: var(--type-label-spacing);
    overflow: hidden;
    flex-shrink: 0;
  }
  .strip-label { color: var(--accent); font-weight: var(--type-label-weight); }
  .strip-empty { color: var(--amber-faint); font-style: italic; font-size: var(--type-caption-size); letter-spacing: var(--type-caption-spacing); }
  .strip-events { display: flex; gap: var(--space-sm); flex: 1; overflow: hidden; }
  .strip-event {
    padding: 1px var(--space-sm);
    border: 1px solid var(--accent);
    color: var(--accent);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(212, 137, 10, 0.06);
    flex-shrink: 0;
  }

  /* Section 3: Recent events log */
  .log {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    box-shadow: var(--sep-depth);
  }
  .log-header {
    padding: var(--space-8) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
    flex-shrink: 0;
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-md) var(--space-lg);
    color: var(--amber-warm);
    font-size: var(--text-sm);
    line-height: 1.5;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  /* Section 4: Persistent state panel */
  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    max-height: 220px;
    overflow-y: auto;
    box-shadow: var(--depth-lift), var(--depth-edge-light);
  }
  .state-header {
    padding: var(--space-8) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    box-shadow: var(--sep-depth);
  }
  .state-body {
    padding: var(--space-md) var(--space-lg) var(--space-14);
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .row.k-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
  }
  .k-row .k { color: var(--amber-dim); }
  .k-row .v { color: var(--amber-warm); font-weight: 600; }
  .k-row .v.enforcement-placeholder { color: var(--amber-faint); font-style: italic; }

  /* Rule source tags */
  .rule-sources {
    margin-top: var(--space-8);
    padding-top: var(--space-8);
  }
  .rs-label {
    display: block;
    color: var(--amber-warm);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    margin-bottom: var(--space-sm);
  }
  .rs-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  /* Scoped rule-source tag — amber-bordered small box per §10.1 style */
  .aegis-rule-source-tag {
    padding: 1px var(--space-sm);
    border: 1px solid var(--accent);
    color: var(--accent);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    background: rgba(212, 137, 10, 0.06);
  }

  /* Skill path — faint italic (§10.1 meta lane: #A87830 faint amber italic) */
  .skill-path-row {
    margin-top: var(--space-sm);
    display: flex;
    align-items: baseline;
    gap: var(--space-8);
    font-size: var(--text-xs);
  }
  .skill-path-label {
    color: var(--amber-dim);
    letter-spacing: 0.04em;
    flex-shrink: 0;
  }
  .skill-path {
    color: var(--amber-faint, #A87830);
    font-style: italic;
    font-size: var(--text-2xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Phase 7.3: quick-action buttons — amber-bordered small boxes per §10.1 */
  .quick-actions {
    display: flex;
    flex-direction: row;
    gap: var(--space-sm);
    margin-top: var(--space-md);
    padding-top: var(--space-8);
  }
  .qa-btn {
    padding: 2px var(--space-8);
    border: 1px solid var(--accent);
    color: var(--accent);
    background: rgba(212, 137, 10, 0.06);
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    cursor: pointer;
    text-transform: uppercase;
    transition: background var(--duration-fast);
  }
  .qa-btn:hover {
    background: rgba(212, 137, 10, 0.14);
  }
  .qa-btn:active {
    background: rgba(212, 137, 10, 0.22);
  }
  /* Error text — §10.1 terminal red lane */
  .qa-error {
    margin-top: var(--space-xs);
    color: var(--term-red);
    font-size: var(--text-2xs);
    font-style: italic;
    letter-spacing: 0.04em;
    word-break: break-all;
  }
</style>
