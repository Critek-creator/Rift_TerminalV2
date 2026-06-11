<script lang="ts">
  // Health tab — cross-project portfolio health dashboard.
  //
  // Subscribes to Category::System bus events with kind prefix "health.".
  // Shows a grid of project health cards with vault staleness, sentinel
  // violations, git status, and overall health badge.
  //
  // Event kinds this tab consumes:
  //   health.portfolio — payload: HealthPayload (full project snapshot)

  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { subscribe, publish, type Envelope } from './bus';
  import { NOTIF_TAB_MIME } from './dragMime';
  import { shouldShow, type SeverityLevel } from './notifFilter';

  interface Props {
    severityThreshold?: SeverityLevel;
    onDragBack?: () => void;
  }

  let { severityThreshold = 'info', onDragBack }: Props = $props();

  interface ProjectGit {
    branch: string;
    ahead: number;
    behind: number;
    uncommitted: number;
  }

  interface ProjectHealth {
    name: string;
    code: string;
    vault_path: string;
    vault_staleness_days: number;
    sentinel_violations: { critical: number; warning: number; info: number };
    git: ProjectGit | null;
  }

  interface HealthPayload {
    projects: ProjectHealth[];
    collected_at: string;
  }

  // Local git status for the current project — polled independently of
  // the portfolio health.portfolio events so users always see uncommitted
  // file counts even before the health collector publishes.
  interface LocalGitStatus {
    not_a_repo: boolean;
    branch: string;
    staged: { path: string; status: string }[];
    modified: { path: string; status: string }[];
    untracked: { path: string; status: string }[];
  }

  let localGit = $state<LocalGitStatus | null>(null);
  let gitPollTimer: ReturnType<typeof setInterval> | undefined;
  let commitRequested = $state(false);
  let copyFeedback = $state(false);

  const uncommittedCount = $derived(
    localGit
      ? localGit.staged.length + localGit.modified.length + localGit.untracked.length
      : 0,
  );

  async function pollLocalGit() {
    try {
      const result = await invoke<LocalGitStatus>('git_status_command');
      if (mounted) localGit = result;
    } catch {
      // Non-fatal — git status may fail if not in a repo
    }
  }

  async function requestCommit() {
    commitRequested = true;
    await publish('system', 'health.action.commit_requested', {
      uncommitted: uncommittedCount,
      branch: localGit?.branch ?? 'unknown',
    });
    setTimeout(() => { commitRequested = false; }, 2000);
  }

  async function copyCommitCmd() {
    const cmd = 'git add -A && git commit -m "WIP: uncommitted changes"';
    try {
      await navigator.clipboard.writeText(cmd);
      copyFeedback = true;
      setTimeout(() => { copyFeedback = false; }, 1500);
    } catch {
      // Fallback: no clipboard API in this context
    }
  }

  const LOG_LIMIT = 200;
  const LIVE_WINDOW_MS = 4000;

  // Monotonic sequence for stable {#each} keys (prevents full DOM teardown
  // on buffer trims when array positions shift).
  let _nextSeq = 0;
  type EnvelopeWithSeq = Envelope & { _seq: number };

  let connected = $state(false);
  let error = $state('');
  let projects = $state<ProjectHealth[]>([]);
  let collectedAt = $state<string | null>(null);
  let events = $state<EnvelopeWithSeq[]>([]);
  let lastTickTs = $state<number>(Date.now());
  let paused = $state(false);
  let unsubscribe: (() => Promise<void>) | undefined;
  let mounted = true;

  const liveEvents = $derived.by(() => {
    const cutoff = lastTickTs - LIVE_WINDOW_MS;
    return events.filter((e) => e.ts >= cutoff);
  });

  const projectCount = $derived(projects.length);
  const criticalProjects = $derived(projects.filter((p) => projectBadge(p) === 'red').length);
  const warningProjects = $derived(projects.filter((p) => projectBadge(p) === 'amber').length);
  const healthyProjects = $derived(projects.filter((p) => projectBadge(p) === 'green').length);

  const totalViolations = $derived(
    projects.reduce((sum, p) => sum + p.sentinel_violations.critical + p.sentinel_violations.warning + p.sentinel_violations.info, 0),
  );

  const lastSeenLabel = $derived.by(() => {
    if (events.length === 0) return '—';
    const last = events[events.length - 1];
    const ageMs = Math.max(0, lastTickTs - last.ts);
    if (ageMs < 1000) return 'just now';
    if (ageMs < 60_000) return `${Math.floor(ageMs / 1000)}s ago`;
    if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
    return `${Math.floor(ageMs / 3_600_000)}h ago`;
  });

  function projectBadge(p: ProjectHealth): 'green' | 'amber' | 'red' {
    if (p.sentinel_violations.critical > 0) return 'red';
    if (p.vault_staleness_days > 14) return 'red';
    if (p.sentinel_violations.warning > 0) return 'amber';
    if (p.vault_staleness_days > 7) return 'amber';
    if (p.git && (p.git.uncommitted > 10 || p.git.behind > 5)) return 'amber';
    return 'green';
  }

  function stalenessColor(days: number): string {
    if (days > 14) return 'var(--term-red)';
    if (days > 7) return 'var(--amber-primary)';
    return 'var(--term-green)';
  }

  function badgeColor(badge: 'green' | 'amber' | 'red'): string {
    switch (badge) {
      case 'green': return 'var(--term-green)';
      case 'amber': return 'var(--amber-primary)';
      case 'red': return 'var(--term-red)';
    }
  }

  function badgeLabel(badge: 'green' | 'amber' | 'red'): string {
    switch (badge) {
      case 'green': return 'HEALTHY';
      case 'amber': return 'WARNING';
      case 'red': return 'CRITICAL';
    }
  }

  function badgeIcon(badge: 'green' | 'amber' | 'red'): string {
    switch (badge) {
      case 'green': return '✓';
      case 'amber': return '◆';
      case 'red': return '⊘';
    }
  }

  function handleEnvelope(env: Envelope) {
    if (paused) return;
    if (!env.kind.startsWith('health.')) return;
    if (!shouldShow(env.kind, severityThreshold)) return;
    events = [...events, { ...env, _seq: _nextSeq++ }];
    if (events.length > LOG_LIMIT * 2) events = events.slice(-LOG_LIMIT);

    if (env.kind === 'health.portfolio') {
      const p = (env.payload ?? {}) as Record<string, unknown>;
      const raw = p as unknown as HealthPayload;
      if (Array.isArray(raw.projects)) {
        projects = raw.projects;
      }
      if (typeof raw.collected_at === 'string') {
        collectedAt = raw.collected_at;
      }
    }

    lastTickTs = Date.now();
  }

  let tickTimer: ReturnType<typeof setInterval> | undefined;

  onMount(async () => {
    try {
      const u = await subscribe({ category: 'system' }, handleEnvelope);
      if (!mounted) {
        void u().catch(() => {});
      } else {
        unsubscribe = u;
        connected = true;
      }
    } catch (err) {
      console.error('[HealthTab] bus_subscribe failed', err);
      error = (err as Error).message || 'Connection failed';
    }
    tickTimer = setInterval(() => { lastTickTs = Date.now(); }, 1000);

    void pollLocalGit();
    gitPollTimer = setInterval(pollLocalGit, 10_000);
  });

  onDestroy(() => {
    mounted = false;
    if (tickTimer) clearInterval(tickTimer);
    if (gitPollTimer) clearInterval(gitPollTimer);
    unsubscribe?.().catch(() => {});
  });

  function formatCollectedAt(iso: string): string {
    try {
      const d = new Date(iso);
      return d.toLocaleTimeString(undefined, { hour12: false });
    } catch {
      return iso;
    }
  }

  function clearEvents(): void {
    events = [];
  }

  function onHandleDragStart(e: DragEvent) {
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData(NOTIF_TAB_MIME, '__promoted_pane__');
      e.dataTransfer.setData('text/plain', '__promoted_pane__');
    }
  }
</script>

<section class="pane">
  {#if onDragBack}
    <div
      class="drag-handle"
      role="button"
      tabindex="0"
      draggable={true}
      ondragstart={onHandleDragStart}
      title="drag back to tab strip to dock"
    >
      <span class="handle-glyph" style="color: var(--term-green); font-size: 14px">⊕</span>
      <span class="handle-title">health</span>
      <button
        type="button"
        class="dock-btn"
        draggable={false}
        onclick={(e) => { e.stopPropagation(); onDragBack?.(); }}
        title="Return to tab strip"
        aria-label="Dock pane back to tab strip"
      >↩ dock</button>
    </div>
  {/if}

  {#if error}
    <div class="error-state">⚠ Bus connection failed: {error}</div>
  {:else if !connected}
    <div class="connecting-state">Connecting…</div>
  {:else}
  <header class="status">
    <span class="title"><span class="icon">⊕</span>HEALTH</span>
    <span class="state">
      {projectCount} project{projectCount === 1 ? '' : 's'}
      {#if criticalProjects > 0}
        · <span class="count-red">{criticalProjects} critical</span>
      {/if}
      {#if warningProjects > 0}
        · <span class="count-amber">{warningProjects} warning</span>
      {/if}
      {#if healthyProjects > 0}
        · <span class="count-green">{healthyProjects} healthy</span>
      {/if}
      · last {lastSeenLabel}
    </span>
    <span class="spacer"></span>
    <button type="button"
      class="ctrl-btn"
      class:active={!paused}
      onclick={() => (paused = !paused)}
      title={paused ? 'resume' : 'pause'}
      aria-label={paused ? 'Resume event stream' : 'Pause event stream'}
    >{paused ? '▶' : '⏸'}</button>
    <button type="button" class="ctrl-btn" onclick={clearEvents} title="clear" aria-label="Clear events">✕</button>
  </header>

  <div class="strip">
    <span class="strip-label">LIVE</span>
    {#if liveEvents.length === 0}
      <span class="strip-empty">◇ no recent health events</span>
    {:else}
      <div class="strip-events">
        {#each liveEvents.slice(0, 10) as e (e._seq)}
          <span class="strip-event">
            ⊕ {e.kind.split('.').pop()}
          </span>
        {/each}
      </div>
    {/if}
  </div>

  <div class="log">
    <div class="log-header">PROJECT HEALTH</div>
    <div class="log-body" aria-live="polite">
      {#if projects.length === 0}
        <div class="empty-card">
          <div class="empty-glyph">⊕</div>
          <div class="empty-title">Portfolio health</div>
          <div class="empty-desc">
            Cross-project health monitoring — vault staleness, sentinel violations,
            and git status across your entire portfolio.
          </div>
          <div class="empty-hint">
            Health collector initializing… Data arrives every 60s.
          </div>
        </div>
      {:else}
        <div class="cards-grid">
          {#each projects as project (project.code)}
            {@const badge = projectBadge(project)}
            <div class="project-card" style="--badge-color: {badgeColor(badge)};">
              <div class="card-header">
                <span class="project-name">{project.name}</span>
                <span class="project-code">{project.code}</span>
                <span class="health-badge" style="color: {badgeColor(badge)}; border-color: {badgeColor(badge)};">
                  {badgeIcon(badge)} {badgeLabel(badge)}
                </span>
              </div>

              <div class="card-body">
                <div class="metric-row">
                  <span class="metric-label">vault staleness</span>
                  <span class="metric-value" style="color: {stalenessColor(project.vault_staleness_days)};">
                    {project.vault_staleness_days}d
                  </span>
                </div>

                <div class="metric-row">
                  <span class="metric-label">sentinel violations</span>
                  <span class="metric-value">
                    {#if project.sentinel_violations.critical > 0}
                      <span class="v-critical">{project.sentinel_violations.critical}C</span>
                    {/if}
                    {#if project.sentinel_violations.warning > 0}
                      <span class="v-warning">{project.sentinel_violations.warning}W</span>
                    {/if}
                    {#if project.sentinel_violations.info > 0}
                      <span class="v-info">{project.sentinel_violations.info}I</span>
                    {/if}
                    {#if project.sentinel_violations.critical === 0 && project.sentinel_violations.warning === 0 && project.sentinel_violations.info === 0}
                      <span class="v-clear">none</span>
                    {/if}
                  </span>
                </div>

                {#if project.git}
                  <div class="metric-row">
                    <span class="metric-label">branch</span>
                    <span class="metric-value git-branch">{project.git.branch}</span>
                  </div>
                  <div class="metric-row">
                    <span class="metric-label">sync</span>
                    <span class="metric-value">
                      {#if project.git.ahead > 0}
                        <span class="git-ahead">↑{project.git.ahead}</span>
                      {/if}
                      {#if project.git.behind > 0}
                        <span class="git-behind">↓{project.git.behind}</span>
                      {/if}
                      {#if project.git.ahead === 0 && project.git.behind === 0}
                        <span class="git-synced">synced</span>
                      {/if}
                    </span>
                  </div>
                  <div class="metric-row">
                    <span class="metric-label">uncommitted</span>
                    <span class="metric-value" style="color: {project.git.uncommitted > 0 ? 'var(--amber-bright)' : 'var(--term-green)'};">
                      {project.git.uncommitted}
                    </span>
                  </div>
                {:else}
                  <div class="metric-row">
                    <span class="metric-label">git</span>
                    <span class="metric-value git-na">n/a</span>
                  </div>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    {#if localGit && !localGit.not_a_repo}
      <div class="state-header">GIT ACTIONS</div>
      <div class="git-actions-body">
        <div class="git-summary">
          <span class="git-branch-label">⎇ {localGit.branch}</span>
          <span class="git-file-count" class:has-changes={uncommittedCount > 0}>
            {uncommittedCount} uncommitted file{uncommittedCount === 1 ? '' : 's'}
          </span>
        </div>
        {#if uncommittedCount > 0}
          <div class="git-file-breakdown">
            {#if localGit.staged.length > 0}
              <span class="git-staged">{localGit.staged.length} staged</span>
            {/if}
            {#if localGit.modified.length > 0}
              <span class="git-modified">{localGit.modified.length} modified</span>
            {/if}
            {#if localGit.untracked.length > 0}
              <span class="git-untracked">{localGit.untracked.length} untracked</span>
            {/if}
          </div>
          <div class="git-btn-row">
            <button type="button"
              class="git-action-btn primary"
              class:requested={commitRequested}
              onclick={requestCommit}
              disabled={commitRequested}
              title="Publish a bus event requesting Claude to commit these files"
            >
              {commitRequested ? '✓ requested' : '⊕ request commit'}
            </button>
            <button type="button"
              class="git-action-btn"
              class:copied={copyFeedback}
              onclick={copyCommitCmd}
              title="Copy a git commit command to clipboard"
            >
              {copyFeedback ? '✓ copied' : '⎘ copy cmd'}
            </button>
          </div>
        {:else}
          <div class="git-clean">✓ working tree clean</div>
        {/if}
      </div>
    {/if}

    <div class="state-header">PORTFOLIO STATUS</div>
    <div class="state-body">
      <div class="k-row">
        <span class="k">projects tracked</span>
        <span class="v">{projectCount}</span>
      </div>
      {#if criticalProjects > 0}
        <div class="k-row"><span class="k">critical</span><span class="v v-critical">{criticalProjects}</span></div>
      {/if}
      {#if warningProjects > 0}
        <div class="k-row"><span class="k">warnings</span><span class="v v-warning">{warningProjects}</span></div>
      {/if}
      <div class="k-row"><span class="k">healthy</span><span class="v v-healthy">{healthyProjects}</span></div>
      <div class="k-row"><span class="k">total violations</span><span class="v">{totalViolations}</span></div>
      <div class="k-row"><span class="k">total events</span><span class="v">{events.length}</span></div>
      {#if collectedAt}
        <div class="k-row"><span class="k">last collection</span><span class="v">{formatCollectedAt(collectedAt)}</span></div>
      {/if}
    </div>
  </footer>
  {/if}
</section>

<style>
  .connecting-state {
    color: var(--amber-faint);
    padding: var(--space-lg) var(--space-14);
    font-style: italic;
    font-size: var(--text-sm);
    letter-spacing: 0.04em;
  }
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    background: var(--bg-base);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-base);
  }

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
  }
  .drag-handle { transition: background var(--duration-base) ease-out; }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: -2px; }
  .drag-handle .handle-glyph {
    color: var(--term-green);
    font-size: var(--text-base);
  }
  .drag-handle .handle-title {
    color: var(--term-green);
    text-transform: uppercase;
  }

  .status {
    height: 36px;
    padding: 0 var(--space-lg);
    background: var(--bg-elevated);
    box-shadow: var(--sep-glow);
    display: flex; align-items: center; gap: var(--space-14);
    color: var(--amber-warm);
  }
  .status .title {
    font-size: var(--type-section-size);
    font-weight: var(--type-section-weight);
    letter-spacing: var(--type-section-spacing);
    color: var(--term-green);
  }
  .status .icon { margin-right: var(--space-8); opacity: 0.85; font-size: var(--text-lg); }
  .status .state {
    font-size: var(--type-caption-size);
    font-weight: var(--type-caption-weight);
    letter-spacing: var(--type-caption-spacing);
    color: var(--amber-dim);
  }
  .status .spacer { flex: 1; }

  .count-red { color: var(--term-red); }
  .count-amber { color: var(--amber-primary); }
  .count-green { color: var(--term-green); }

  .ctrl-btn {
    background: none; border: 1px solid var(--border-subtle);
    color: var(--amber-warm); padding: 1px var(--space-8);
    font-family: inherit; font-size: var(--text-xs); cursor: pointer;
    transition: background var(--duration-base), border-color var(--duration-base);
  }
  .ctrl-btn:hover { background: var(--bg-hover); border-color: var(--amber-faint); }
  .ctrl-btn:focus-visible { outline: 1px solid var(--amber-warm); outline-offset: 1px; }
  .ctrl-btn.active { border-color: var(--term-green); color: var(--term-green); }

  .strip {
    height: 26px;
    padding: 0 var(--space-14);
    box-shadow: var(--sep-depth);
    display: flex; align-items: center; gap: var(--space-14);
    background: linear-gradient(to bottom, rgba(79, 232, 85, 0.05), transparent);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    overflow: hidden;
  }
  .strip-label { color: var(--term-green); font-weight: 700; }
  .strip-empty { color: var(--amber-dim); font-size: var(--type-caption-size); font-style: italic; letter-spacing: var(--type-caption-spacing); }
  .strip-events { display: flex; gap: var(--space-sm); flex: 1; overflow: hidden; }
  .strip-event {
    padding: 1px var(--space-sm);
    border: 1px solid var(--term-green);
    color: var(--term-green);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    white-space: nowrap;
    background: rgba(79, 232, 85, 0.06);
    flex-shrink: 0;
  }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
    min-width: 0;
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
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    min-width: 0;
    padding: var(--space-md) var(--space-lg);
    font-size: var(--text-sm);
    line-height: 1.5;
    box-shadow: var(--depth-inset);
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .error-state {
    color: var(--term-red);
    padding: var(--space-12) var(--space-lg);
    font-size: var(--type-body-size);
    letter-spacing: var(--type-body-spacing);
    background: var(--bg-red-tint);
    box-shadow: var(--sep-depth);
  }

  .empty-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-8);
    padding: var(--space-2xl) var(--space-lg);
    text-align: center;
    min-height: 120px;
  }
  .empty-glyph {
    font-size: var(--text-2xl);
    color: var(--term-green);
    opacity: 0.4;
  }
  .empty-title {
    color: var(--amber-dim);
    font-size: var(--type-body-size);
    font-weight: var(--type-body-weight);
    letter-spacing: var(--type-body-spacing);
  }
  .empty-desc {
    font-size: var(--type-caption-size);
    line-height: 1.6;
    max-width: 320px;
    color: var(--amber-dim);
    letter-spacing: var(--type-caption-spacing);
  }
  .empty-hint {
    font-size: var(--type-caption-size);
    font-style: italic;
    color: var(--amber-faint);
    letter-spacing: var(--type-caption-spacing);
  }

  .cards-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-md);
  }

  .project-card {
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    display: flex;
    flex-direction: column;
    transition: border-color var(--duration-base) ease-out, background var(--duration-base) ease-out;
  }
  .project-card:hover {
    border-color: var(--badge-color, var(--amber-faint));
    background: var(--bg-hover);
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-8) var(--space-12);
    border-bottom: 1px solid var(--border-subtle);
  }
  .project-name {
    font-weight: 700;
    font-size: var(--text-sm);
    letter-spacing: 0.04em;
    color: var(--amber-warm);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .project-code {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--amber-faint);
    text-transform: uppercase;
  }
  .health-badge {
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    padding: 1px var(--space-sm);
    border: 1px solid;
    white-space: nowrap;
  }

  .card-body {
    padding: var(--space-8) var(--space-12);
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .metric-row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
  }
  .metric-label {
    color: var(--amber-dim);
    font-weight: 600;
  }
  .metric-value {
    color: var(--amber-warm);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    display: flex;
    gap: var(--space-sm);
    align-items: baseline;
  }

  .v-critical { color: var(--term-red); font-weight: 700; }
  .v-warning { color: var(--amber-primary); }
  .v-info { color: var(--term-cyan); }
  .v-clear { color: var(--term-green); font-weight: 400; }

  .git-branch {
    color: var(--term-cyan);
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .git-ahead { color: var(--amber-bright); }
  .git-behind { color: var(--amber-primary); }
  .git-synced { color: var(--term-green); font-weight: 400; }
  .git-na { color: var(--amber-faint); font-style: italic; font-weight: 400; }

  .state-panel { box-shadow: var(--depth-lift); flex-shrink: 0; }

  .git-actions-body {
    padding: var(--space-12) var(--space-lg);
    display: flex;
    flex-direction: column;
    gap: var(--space-8);
    border-bottom: 1px solid var(--border-subtle);
  }
  .git-summary {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-size: var(--text-sm);
  }
  .git-branch-label {
    color: var(--term-cyan);
    font-weight: 600;
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
  }
  .git-file-count {
    color: var(--term-green);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .git-file-count.has-changes {
    color: var(--amber-bright);
  }
  .git-file-breakdown {
    display: flex;
    gap: var(--space-md);
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
  }
  .git-staged { color: var(--term-green); }
  .git-modified { color: var(--amber-primary); }
  .git-untracked { color: var(--amber-dim); }

  .git-btn-row {
    display: flex;
    gap: var(--space-8);
    margin-top: var(--space-xs);
  }
  .git-action-btn {
    flex: 1;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    padding: var(--space-xs) var(--space-md);
    cursor: pointer;
    border-radius: var(--radius-md, 4px);
    transition: color var(--duration-base), border-color var(--duration-base),
                background var(--duration-base), box-shadow var(--duration-base);
  }
  .git-action-btn:hover {
    color: var(--amber-bright);
    border-color: var(--amber-primary);
    background: var(--bg-hover);
    box-shadow: 0 0 6px rgba(212, 137, 10, 0.2);
  }
  .git-action-btn:active {
    background: rgba(255, 168, 38, 0.1);
  }
  .git-action-btn:focus-visible {
    outline: 1px solid var(--amber-primary);
    outline-offset: 1px;
  }
  .git-action-btn.primary {
    border-color: var(--term-green);
    color: var(--term-green);
    background: rgba(79, 232, 85, 0.06);
  }
  .git-action-btn.primary:hover {
    box-shadow: 0 0 8px rgba(79, 232, 85, 0.25);
    background: rgba(79, 232, 85, 0.1);
    border-color: var(--term-green);
    color: var(--term-green);
  }
  .git-action-btn.requested,
  .git-action-btn.copied {
    color: var(--term-green);
    border-color: var(--term-green);
    cursor: default;
  }
  .git-action-btn:disabled {
    opacity: 0.7;
    cursor: default;
  }

  .git-clean {
    color: var(--term-green);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    opacity: 0.8;
  }
  .state-header {
    padding: var(--space-8) var(--space-lg);
    color: var(--amber-faint);
    font-size: var(--type-label-size);
    font-weight: var(--type-label-weight);
    letter-spacing: var(--type-label-spacing);
    text-transform: uppercase;
    background: var(--bg-surface);
    box-shadow: var(--sep-depth);
  }
  .state-body { padding: var(--space-md) var(--space-lg); }
  .k-row {
    display: flex;
    justify-content: space-between;
    padding: 2px 0;
    font-size: var(--text-sm);
  }
  .k { color: var(--amber-dim); }
  .v { color: var(--amber-warm); font-weight: 600; font-variant-numeric: tabular-nums; }
  .v-critical { color: var(--term-red); font-weight: 700; }
  .v-warning { color: var(--amber-primary); }
  .v-healthy { color: var(--term-green); }
</style>
