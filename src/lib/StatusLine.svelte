<script lang="ts">
  // §10.2 — two-row status line. Color-block segments, dark text on
  // category-tinted backgrounds. All values bold.
  //
  // Row 1: DIR / MODEL / THINKING / CTX% / SESSION / SKILL
  // Row 2: GIT / REPO / SESSION USE% / WEEK% / EFFORT
  //
  // Segment data sources:
  //   * dir / git / repo       → live via translators/status.rs (5s poll)
  //   * thinking               → CC StatusJSON thinking.effort level
  //   * skill / effort         → live via Aegis integration (feature-gated)
  //   * model / ctx / session  → CC StatusJSON via cc_status translator
  //   * sessionUse / week      → CC StatusJSON rate_limits
  //
  // Phase 8.7g.2 color families:
  //   GREEN  — env locale (DIR, GIT, REPO)
  //   CYAN   — model identity (MODEL, THINKING)
  //   BLUE   — usage metrics (CTX%, SESSION USE%, WEEK%)
  //   AMBER  — aegis state (SKILL, EFFORT)
  //   PURPLE — session/clock (SESSION)
  //
  // Visibility controlled by StatusLineConfig per-segment toggles.

  import type { StatusLineConfig } from './riftConfig';
  import ProfilePicker from './ProfilePicker.svelte';
  import ModelIndicator from './ModelIndicator.svelte';
  import { llmRouting } from './llmRouting.svelte';
  import { commandFailureStore } from './commandFailureStore.svelte';

  interface Props {
    dir?: string;
    model?: string;
    thinking?: string;
    ctx?: string;
    session?: string;
    skill?: string;
    effort?: string;
    git?: string;
    repo?: string;
    sessionUse?: string;
    week?: string;
    /** Focused pane's foreground-process-tree CPU%, e.g. "12.3%". Blank = unavailable. */
    cpu?: string;
    /** Focused pane's foreground-process-tree resident memory, e.g. "184 MB". Blank = unavailable. */
    ram?: string;
    visibility?: StatusLineConfig;
    /** Open the command palette pre-filtered to model switching (hot-swap). */
    onmodelswap?: () => void;
  }

  let {
    dir = '—',
    model = '—',
    thinking = '—',
    ctx = '—',
    session = '—',
    skill = '—',
    effort = '—',
    git = '—',
    repo = '—',
    sessionUse = '—',
    week = '—',
    cpu = '',
    ram = '',
    visibility,
    onmodelswap,
  }: Props = $props();

  const show = $derived({
    dir: visibility?.show_dir ?? true,
    model: visibility?.show_model ?? true,
    thinking: visibility?.show_thinking ?? true,
    ctx: visibility?.show_ctx ?? true,
    session: visibility?.show_session ?? true,
    skill: visibility?.show_skill ?? true,
    effort: visibility?.show_effort ?? true,
    git: visibility?.show_git ?? true,
    repo: visibility?.show_repo ?? true,
    sessionUse: visibility?.show_session_use ?? true,
    week: visibility?.show_week ?? true,
    cpu: visibility?.show_cpu ?? true,
    ram: visibility?.show_ram ?? true,
    cost: visibility?.show_cost ?? true,
  });

  let costLabel = $derived(llmRouting.formatCost(llmRouting.sessionCostUsd));

  function override(key: string): string | undefined {
    return visibility?.color_overrides?.[key];
  }

  // Bottom-chrome quick access to Claude /commands — opens the unified command
  // palette in /-mode (App.svelte listens for this event). The /commands
  // launcher used to be a hidden Ctrl+Shift+P surface; this makes it visible.
  function openCommands(): void {
    window.dispatchEvent(new CustomEvent('rift:open-commands'));
  }

  // Phase 5 / R1.5 — open the persistent command-failures issues list. The
  // chip lights when there are unacknowledged shell failures; the list survives
  // scroll-out (unlike the per-line badge) and explains each via a local model.
  const failureCount = $derived(commandFailureStore.count);
  const unackFailures = $derived(commandFailureStore.unacknowledgedCount);
  function openFailures(): void {
    window.dispatchEvent(new CustomEvent('rift:open-failures'));
  }
</script>

<footer class="statusline" role="status" aria-live="polite" aria-label="Terminal status">
  <div class="row">
    {#if show.dir}
      <div class="seg dir" style:background={override('dir')}>
        <span class="label">DIR</span><span class="value">{dir}</span>
      </div>
    {/if}
    {#if show.model}
      <div class="seg model" style:background={override('model')}>
        <span class="label">MODEL</span><span class="value">{model}</span>
        <ModelIndicator />
        {#if onmodelswap}
          <button
            type="button"
            class="swap-btn"
            onclick={onmodelswap}
            title="Switch / hot-swap model (Ctrl+Shift+M)"
            aria-label="Switch model"
          >⇄ swap</button>
        {/if}
      </div>
    {/if}
    {#if show.thinking}
      <div class="seg thinking" style:background={override('thinking')}>
        <span class="label">THINKING</span><span class="value">{thinking}</span>
      </div>
    {/if}
    {#if show.ctx}
      <div class="seg ctx" style:background={override('ctx')}>
        <span class="label">CTX</span><span class="value">{ctx}</span>
      </div>
    {/if}
    {#if show.session}
      <div class="seg session" style:background={override('session')}>
        <span class="label">SESSION</span><span class="value">{session}</span>
      </div>
    {/if}
    {#if show.skill}
      <div class="seg skill" style:background={override('skill')}>
        <span class="label">SKILL</span><span class="value">{skill}</span>
      </div>
    {/if}
    <div class="seg spacer"></div>
    <button
      type="button"
      class="issues-btn"
      class:lit={unackFailures > 0}
      onclick={openFailures}
      title="Command failures — click to list and explain (local model)"
      aria-label={`Open command failures list${failureCount > 0 ? ` (${failureCount})` : ''}`}
    >
      <span class="issues-mark" aria-hidden="true">✗</span>
      <span class="issues-label">issues</span>
      {#if failureCount > 0}<span class="issues-count">{failureCount > 99 ? '99+' : failureCount}</span>{/if}
    </button>
    <button
      type="button"
      class="cmd-btn"
      onclick={openCommands}
      title="Claude /commands — quick access (Ctrl+Shift+P)"
      aria-label="Open Claude commands"
    >/ commands</button>
  </div>
  <div class="row">
    {#if show.git}
      <div class="seg git" style:background={override('git')}>
        <span class="label">GIT</span><span class="value">{git}</span>
      </div>
    {/if}
    {#if show.repo}
      <div class="seg repo" style:background={override('repo')}>
        <span class="label">REPO</span><span class="value">{repo}</span>
      </div>
    {/if}
    {#if show.sessionUse}
      <div class="seg session-use" style:background={override('session_use')}>
        <span class="label">USE</span><span class="value">{sessionUse}</span>
      </div>
    {/if}
    {#if show.week}
      <div class="seg week" style:background={override('week')}>
        <span class="label">WEEK</span><span class="value">{week}</span>
      </div>
    {/if}
    {#if show.effort}
      <div class="seg effort" style:background={override('effort')}>
        <span class="label">EFFORT</span><span class="value">{effort}</span>
      </div>
    {/if}
    {#if show.cpu && cpu}
      <div class="seg cpu" style:background={override('cpu')}>
        <span class="label">CPU</span><span class="value">{cpu}</span>
      </div>
    {/if}
    {#if show.ram && ram}
      <div class="seg ram" style:background={override('ram')}>
        <span class="label">RAM</span><span class="value">{ram}</span>
      </div>
    {/if}
    {#if show.cost && llmRouting.sessionCostUsd > 0}
      <div class="seg cost" style:background={override('cost')}>
        <span class="label">COST</span><span class="value">{costLabel}</span>
      </div>
    {/if}
    <div class="seg spacer"></div>
    <div class="seg profile-seg">
      <ProfilePicker />
    </div>
  </div>
</footer>

<style>
  .statusline {
    flex-shrink: 0;
    background-color: var(--bg-surface);
    background-image: var(--grain);
    border-top: 1px solid var(--border-active);
    font-size: var(--text-sm);
    line-height: 1;
    user-select: none;
  }
  .row {
    display: flex;
    align-items: stretch;
    height: 22px;
  }
  .row + .row { border-top: 1px solid var(--border-subtle); }

  .seg {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: 0 var(--space-md);
    border-right: 1px solid rgba(255, 168, 38, 0.12);
    white-space: nowrap;
    min-width: 0;
    flex-shrink: 1;
  }
  .seg:last-child { border-right: none; }

  .label {
    color: rgba(0, 0, 0, 0.65);
    font-weight: 700;
    font-size: var(--text-2xs);
    letter-spacing: 0.1em;
  }
  .value {
    color: rgba(0, 0, 0, 0.92);
    font-variant-numeric: tabular-nums;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .spacer {
    flex: 1;
    border-right: none;
    background: transparent !important;
  }

  /* GREEN family — env locale */
  .dir    { background: var(--status-green-bright); }
  .git    { background: var(--status-green-mid); }
  .repo   { background: var(--status-green-dim); }

  /* CYAN — model identity + thinking */
  .model    { background: var(--status-cyan-bright); }

  /* Hot-swap affordance inside the MODEL segment — dark text on the cyan
     block, matching the segment's inverted color scheme. */
  .swap-btn {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin-left: 4px;
    padding: 1px 5px;
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.05em;
    color: rgba(0, 0, 0, 0.78);
    background: rgba(0, 0, 0, 0.12);
    border: 1px solid rgba(0, 0, 0, 0.28);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease-out);
  }
  .swap-btn:hover { background: rgba(0, 0, 0, 0.22); }
  .swap-btn:focus-visible {
    outline: 2px solid rgba(0, 0, 0, 0.5);
    outline-offset: 1px;
  }

  /* /commands quick-access — sits on the statusline ground (not inside a tinted
     segment), so it reads as an amber chip rather than dark-on-tint. */
  .cmd-btn {
    align-self: center;
    margin-right: var(--space-sm);
    padding: 1px 8px;
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.05em;
    color: var(--amber-warm);
    background: transparent;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
  }
  .cmd-btn:hover {
    color: var(--amber-bright);
    background: rgba(255, 200, 64, 0.08);
    border-color: var(--amber-dim);
  }
  .cmd-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }

  /* Issues chip — persistent shell-failure list trigger, sits left of
     /commands. Faint by default; lights red when there are unacknowledged
     failures, mirroring the ambient ERR indicator. */
  .issues-btn {
    align-self: center;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-right: var(--space-sm);
    padding: 1px 8px;
    font-family: var(--font-family);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.05em;
    color: var(--amber-faint);
    background: transparent;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    cursor: pointer;
    white-space: nowrap;
    transition: color var(--duration-fast) var(--ease-out), background var(--duration-fast) var(--ease-out), border-color var(--duration-fast) var(--ease-out);
  }
  .issues-btn:hover {
    color: var(--amber-warm);
    background: rgba(255, 200, 64, 0.06);
    border-color: var(--amber-dim);
  }
  .issues-btn:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }
  .issues-mark { color: var(--amber-faint); transition: color var(--duration-fast) var(--ease-out); }
  .issues-count {
    font-variant-numeric: tabular-nums;
    color: var(--amber-warm);
    background: rgba(255, 200, 64, 0.10);
    border-radius: var(--radius-sm);
    padding: 0 4px;
  }
  /* Lit — unacknowledged failures present. */
  .issues-btn.lit { color: var(--term-white, #e8e4d8); border-color: rgba(255, 72, 72, 0.5); }
  .issues-btn.lit .issues-mark { color: var(--term-red); }
  .issues-btn.lit .issues-count { color: #ff8a8a; background: rgba(255, 72, 72, 0.14); }
  .thinking { background: var(--status-cyan-dim); }

  /* BLUE family — usage metrics */
  .ctx         { background: var(--status-blue-mid); }
  .session-use { background: var(--status-blue-bright); }
  .week        { background: var(--status-blue-dim); }
  .cpu         { background: var(--status-blue-mid); }
  .ram         { background: var(--status-blue-dim); }

  /* AMBER family — Aegis state */
  .skill  { background: var(--amber-primary); }
  .effort { background: var(--amber-dim); }

  /* PURPLE — session clock */
  .session { background: var(--status-time); }

  /* AMBER-DIM — cost ticker */
  .cost { background: var(--amber-dim, rgba(168, 120, 48, 0.5)); }

  .profile-seg {
    flex-shrink: 0;
    padding: 0;
    background: transparent;
    position: relative;
  }
</style>
