<script lang="ts">
  // Phase 8.7i — Git tab. Branch + ahead/behind + working-tree changes
  // + last commit, polled every 5s.
  //
  // Backend: `git_status_command` shells out to `git -C <root> status` and
  // `git log -1`. If the project root is not a git repo, `not_a_repo` is
  // true and we render a friendly empty state instead of an error.
  //
  // Click any modified/staged/untracked path → opens it in the Viewer
  // popout (read-only — staging UI is a future enhancement).

  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { popouts } from './popouts.svelte';
  import { NOTIF_TAB_MIME } from './dragMime';

  interface Props {
    onDragBack?: () => void;
  }

  let { onDragBack }: Props = $props();

  interface GitFileEntry {
    path: string;
    status: string;
  }
  interface GitCommit {
    hash: string;
    short_hash: string;
    subject: string;
    author: string;
    iso_date: string;
  }
  interface GitStatus {
    not_a_repo: boolean;
    branch: string;
    upstream: string;
    ahead: number;
    behind: number;
    staged: GitFileEntry[];
    modified: GitFileEntry[];
    untracked: GitFileEntry[];
    last_commit: GitCommit | null;
  }

  const POLL_INTERVAL_MS = 5000;

  let snapshot = $state<GitStatus | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let lastPollTs = $state<number | null>(null);
  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let mounted = true;

  // Phase 8.7j — git mutating actions (fetch / pull / push / commit).
  type GitActionKind = 'fetch' | 'pull' | 'push' | 'commit-all';
  interface GitActionResult {
    success: boolean;
    stdout: string;
    stderr: string;
    exit_code: number;
  }
  let actionRunning = $state<GitActionKind | null>(null);
  let lastActionLabel = $state<string | null>(null);
  let lastActionResult = $state<GitActionResult | null>(null);
  let lastActionFailed = $state(false);
  let commitOpen = $state(false);
  let commitMessage = $state('');

  async function runAction(kind: GitActionKind, message?: string) {
    actionRunning = kind;
    lastActionResult = null;
    lastActionFailed = false;
    try {
      const result = await invoke<GitActionResult>('git_action_command', {
        action: kind,
        message: message ?? null,
      });
      lastActionResult = result;
      lastActionFailed = !result.success;
      lastActionLabel = kind;
      if (result.success) {
        // Refresh the snapshot to reflect the new state.
        await poll();
      }
    } catch (err) {
      lastActionFailed = true;
      lastActionResult = {
        success: false,
        stdout: '',
        stderr: String(err),
        exit_code: -1,
      };
      lastActionLabel = kind;
    } finally {
      actionRunning = null;
    }
  }

  async function doCommit() {
    if (!commitMessage.trim()) return;
    const msg = commitMessage.trim();
    await runAction('commit-all', msg);
    if (lastActionResult?.success) {
      commitMessage = '';
      commitOpen = false;
    }
  }

  const cleanWorkingTree = $derived.by(() => {
    if (!snapshot || snapshot.not_a_repo) return false;
    return (
      snapshot.staged.length === 0
      && snapshot.modified.length === 0
      && snapshot.untracked.length === 0
    );
  });

  const totalChanges = $derived.by(() => {
    if (!snapshot) return 0;
    return snapshot.staged.length + snapshot.modified.length + snapshot.untracked.length;
  });

  async function poll() {
    loading = true;
    error = null;
    try {
      const result = await invoke<GitStatus>('git_status_command');
      if (!mounted) return;
      snapshot = result;
      lastPollTs = Date.now();
    } catch (err) {
      if (!mounted) return;
      error = String(err);
      console.error('[GitTab] git_status_command failed', err);
    } finally {
      if (mounted) loading = false;
    }
  }

  onMount(() => {
    void poll();
    pollTimer = setInterval(() => {
      void poll();
    }, POLL_INTERVAL_MS);
  });

  onDestroy(() => {
    mounted = false;
    if (pollTimer) clearInterval(pollTimer);
  });

  function openFile(entry: GitFileEntry) {
    popouts.summon({
      content: { kind: 'viewer', path: entry.path },
    });
  }

  function formatPollLabel(): string {
    if (loading && lastPollTs === null) return 'loading…';
    if (error) return `error: ${error}`;
    if (lastPollTs === null) return 'awaiting first poll';
    const ageMs = Date.now() - lastPollTs;
    if (ageMs < 1000) return 'just now';
    if (ageMs < 60_000) return `${Math.floor(ageMs / 1000)}s ago`;
    if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
    return `${Math.floor(ageMs / 3_600_000)}h ago`;
  }

  function formatCommitDate(iso: string): string {
    if (!iso) return '';
    try {
      const d = new Date(iso);
      const ageMs = Date.now() - d.getTime();
      if (ageMs < 60_000) return 'just now';
      if (ageMs < 3_600_000) return `${Math.floor(ageMs / 60_000)}m ago`;
      if (ageMs < 86_400_000) return `${Math.floor(ageMs / 3_600_000)}h ago`;
      if (ageMs < 30 * 86_400_000) return `${Math.floor(ageMs / 86_400_000)}d ago`;
      return d.toLocaleDateString();
    } catch {
      return iso;
    }
  }

  function statusGlyph(s: string): string {
    switch (s) {
      case 'M': return 'M';
      case 'A': return 'A';
      case 'D': return 'D';
      case 'R': return 'R';
      case 'C': return 'C';
      case '?': return '?';
      default: return s;
    }
  }
  function statusColor(s: string): string {
    switch (s) {
      case 'M': return 'var(--amber-bright)';
      case 'A': return 'var(--term-green, #4FE855)';
      case 'D': return 'var(--term-red)';
      case 'R': return 'var(--term-purple)';
      case 'C': return 'var(--term-cyan)';
      case '?': return 'var(--amber-faint)';
      default: return 'var(--amber-warm)';
    }
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
      <span class="handle-glyph">↙</span>
      <span class="handle-title">git</span>
      <span class="handle-hint">drag to dock</span>
    </div>
  {/if}

  <header class="status">
    <span class="title"><span class="icon">⎇</span>GIT</span>
    {#if snapshot && !snapshot.not_a_repo}
      <span class="branch">{snapshot.branch || '(detached)'}</span>
      {#if snapshot.upstream}
        <span class="upstream">→ {snapshot.upstream}</span>
      {/if}
      {#if snapshot.ahead > 0}
        <span class="chip chip-ahead">↑{snapshot.ahead}</span>
      {/if}
      {#if snapshot.behind > 0}
        <span class="chip chip-behind">↓{snapshot.behind}</span>
      {/if}
      {#if cleanWorkingTree}
        <span class="chip chip-clean">clean</span>
      {/if}
    {/if}
    <span class="spacer"></span>
    <span class="state">{formatPollLabel()}</span>
    <button type="button" class="ctl-btn" onclick={poll} disabled={loading}>
      {loading ? '…' : 'refresh'}
    </button>
  </header>

  {#if snapshot && !snapshot.not_a_repo}
    <div class="actions">
      <button
        type="button"
        class="ctl-btn"
        onclick={() => runAction('fetch')}
        disabled={actionRunning !== null}
        title="git fetch --prune"
      >{actionRunning === 'fetch' ? 'fetching…' : 'fetch'}</button>
      <button
        type="button"
        class="ctl-btn"
        onclick={() => runAction('pull')}
        disabled={actionRunning !== null}
        title="git pull --ff-only"
      >{actionRunning === 'pull' ? 'pulling…' : 'pull'}</button>
      <button
        type="button"
        class="ctl-btn"
        onclick={() => runAction('push')}
        disabled={actionRunning !== null}
        title="git push"
      >{actionRunning === 'push' ? 'pushing…' : 'push'}</button>
      <button
        type="button"
        class="ctl-btn"
        class:active={commitOpen}
        onclick={() => (commitOpen = !commitOpen)}
        disabled={actionRunning !== null}
        title="stage all + commit"
      >{commitOpen ? 'cancel' : 'commit'}</button>
      {#if lastActionResult}
        <span class="action-banner" class:fail={lastActionFailed}>
          {lastActionLabel}: {lastActionFailed ? 'failed' : 'ok'}
          {#if lastActionResult.stderr.trim() || lastActionResult.stdout.trim()}
            <span class="action-detail">
              {(lastActionResult.stderr.trim() || lastActionResult.stdout.trim()).slice(0, 160)}
            </span>
          {/if}
          <button
            type="button"
            class="banner-close"
            onclick={() => { lastActionResult = null; lastActionFailed = false; }}
            aria-label="dismiss"
          >×</button>
        </span>
      {/if}
    </div>

    {#if commitOpen}
      <div class="commit-form">
        <textarea
          class="commit-input"
          bind:value={commitMessage}
          placeholder="commit message — `git add -A && git commit -m …`"
          rows="2"
          disabled={actionRunning !== null}
        ></textarea>
        <div class="commit-buttons">
          <button
            type="button"
            class="ctl-btn"
            onclick={() => { commitOpen = false; commitMessage = ''; }}
            disabled={actionRunning !== null}
          >cancel</button>
          <button
            type="button"
            class="ctl-btn primary"
            onclick={doCommit}
            disabled={actionRunning !== null || !commitMessage.trim()}
          >{actionRunning === 'commit-all' ? 'committing…' : 'commit'}</button>
        </div>
      </div>
    {/if}
  {/if}

  <div class="strip">
    <span class="strip-label">CHANGES</span>
    {#if !snapshot || snapshot.not_a_repo}
      <span class="strip-empty">(not a git repository)</span>
    {:else if totalChanges === 0}
      <span class="strip-empty">(working tree clean)</span>
    {:else}
      {#if snapshot.staged.length > 0}
        <span class="chip chip-staged">{snapshot.staged.length} staged</span>
      {/if}
      {#if snapshot.modified.length > 0}
        <span class="chip chip-modified">{snapshot.modified.length} modified</span>
      {/if}
      {#if snapshot.untracked.length > 0}
        <span class="chip chip-untracked">{snapshot.untracked.length} untracked</span>
      {/if}
    {/if}
  </div>

  <div class="log">
    <div class="log-header">FILES · click to open</div>
    <div class="log-body">
      {#if error}
        <div class="empty error">{error}</div>
      {:else if !snapshot}
        <div class="empty">loading…</div>
      {:else if snapshot.not_a_repo}
        <div class="empty">
          this project root is not a git working tree — no status to display
        </div>
      {:else if totalChanges === 0}
        <div class="empty">working tree is clean</div>
      {:else}
        {#if snapshot.staged.length > 0}
          <div class="group-header">staged</div>
          {#each snapshot.staged as f, i (f.path + i + 's')}
            <button
              type="button"
              class="row"
              onclick={() => openFile(f)}
              title="open {f.path}"
            >
              <span class="file-status" style="color: {statusColor(f.status)};">
                {statusGlyph(f.status)}
              </span>
              <span class="path">{f.path}</span>
            </button>
          {/each}
        {/if}
        {#if snapshot.modified.length > 0}
          <div class="group-header">modified</div>
          {#each snapshot.modified as f, i (f.path + i + 'm')}
            <button
              type="button"
              class="row"
              onclick={() => openFile(f)}
              title="open {f.path}"
            >
              <span class="file-status" style="color: {statusColor(f.status)};">
                {statusGlyph(f.status)}
              </span>
              <span class="path">{f.path}</span>
            </button>
          {/each}
        {/if}
        {#if snapshot.untracked.length > 0}
          <div class="group-header">untracked</div>
          {#each snapshot.untracked as f, i (f.path + i + 'u')}
            <button
              type="button"
              class="row"
              onclick={() => openFile(f)}
              title="open {f.path}"
            >
              <span class="file-status" style="color: {statusColor(f.status)};">?</span>
              <span class="path">{f.path}</span>
            </button>
          {/each}
        {/if}
      {/if}
    </div>
  </div>

  <footer class="state-panel">
    <div class="state-header">LAST COMMIT</div>
    <div class="state-body">
      {#if snapshot?.last_commit}
        <div class="commit-row">
          <span class="commit-hash">{snapshot.last_commit.short_hash}</span>
          <span class="commit-subject">{snapshot.last_commit.subject}</span>
        </div>
        <div class="commit-meta">
          <span class="commit-author">{snapshot.last_commit.author}</span>
          <span class="commit-sep">·</span>
          <span class="commit-date">{formatCommitDate(snapshot.last_commit.iso_date)}</span>
        </div>
      {:else}
        <div class="empty">no commits yet</div>
      {/if}
    </div>
  </footer>
</section>

<style>
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-base);
    color: var(--amber-warm);
    font-family: var(--font-family);
    font-size: var(--text-base);
  }

  .drag-handle {
    height: 26px;
    padding: 0 12px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: grab;
    user-select: none;
    color: var(--amber-warm);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    font-weight: 700;
    transition: background var(--duration-base) ease-out;
  }
  .drag-handle:active { cursor: grabbing; }
  .drag-handle:hover { background: var(--bg-hover); }
  .drag-handle .handle-glyph {
    color: var(--amber-bright);
    font-size: var(--text-base);
    text-shadow: var(--glow-amber-faint);
  }
  .drag-handle .handle-title {
    color: var(--amber-bright);
    text-transform: uppercase;
  }
  .drag-handle .handle-hint {
    margin-left: auto;
    color: var(--amber-faint);
    font-style: italic;
    font-weight: 400;
    letter-spacing: 0.04em;
  }

  .status {
    height: 30px;
    padding: 0 14px;
    background: var(--bg-elevated);
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
    display: flex; align-items: center; gap: 10px;
    color: var(--amber-warm);
    font-size: var(--text-sm); letter-spacing: 0.1em; font-weight: 700;
  }
  .status .title {
    color: var(--amber-bright);
    text-shadow: var(--glow-amber-faint);
  }
  .status .icon { margin-right: 8px; opacity: 0.85; }
  .status .branch {
    color: var(--amber-warm);
    font-weight: 700;
    letter-spacing: 0.06em;
  }
  .status .upstream {
    color: var(--amber-faint);
    font-weight: 400;
    font-style: italic;
    letter-spacing: 0.04em;
  }
  .status .state {
    color: var(--amber-dim);
    font-weight: 400;
    letter-spacing: 0.04em;
    font-size: var(--text-xs);
  }
  .status .spacer { flex: 1; }
  .ctl-btn {
    background: transparent;
    border: 1px solid var(--amber-faint);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-2xs);
    letter-spacing: 0.1em;
    font-weight: 700;
    padding: 2px 8px;
    cursor: pointer;
    text-transform: uppercase;
    transition: color var(--duration-base) ease-out, background var(--duration-base) ease-out, border-color var(--duration-base) ease-out, box-shadow var(--duration-base) ease-out, opacity var(--duration-base) ease-out;
  }
  .ctl-btn:hover:not(:disabled) {
    border-color: var(--amber-bright);
    color: var(--amber-bright);
  }
  .ctl-btn:focus-visible {
    outline: 1px solid var(--amber-bright);
    outline-offset: 1px;
  }
  .ctl-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .ctl-btn.active {
    background: var(--amber-bright);
    border-color: var(--amber-bright);
    color: var(--bg-base);
  }
  .ctl-btn.primary {
    background: var(--amber-bright);
    border-color: var(--amber-bright);
    color: var(--bg-base);
  }
  .ctl-btn.primary:hover:not(:disabled) {
    box-shadow: var(--glow-amber-faint);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-elevated);
    flex-wrap: wrap;
  }
  .action-banner {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 2px 8px;
    border: 1px solid var(--term-green, #4FE855);
    color: var(--term-green, #4FE855);
    font-size: var(--text-2xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    font-weight: 700;
    margin-left: auto;
    max-width: 60%;
  }
  .action-banner.fail {
    border-color: var(--term-red);
    color: var(--term-red);
  }
  .action-detail {
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0.02em;
    color: var(--amber-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .banner-close {
    background: transparent;
    border: none;
    color: inherit;
    font-size: var(--text-base);
    line-height: 1;
    cursor: pointer;
    padding: 0 2px;
    transition: color var(--duration-base) ease-out, opacity var(--duration-base) ease-out;
  }

  .commit-form {
    padding: 8px 14px 10px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-panel);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .commit-input {
    background: var(--bg-base);
    border: 1px solid var(--amber-faint);
    color: var(--amber-warm);
    font-family: inherit;
    font-size: var(--text-sm);
    padding: 6px 8px;
    resize: vertical;
    min-height: 32px;
  }
  .commit-input:focus {
    outline: none;
    border-color: var(--amber-bright);
    box-shadow: var(--glow-amber-faint);
  }
  .commit-buttons {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border: 1px solid var(--amber-faint);
    color: var(--amber-warm);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .chip-ahead { border-color: var(--term-cyan); color: var(--term-cyan); }
  .chip-behind { border-color: var(--term-red); color: var(--term-red); }
  .chip-clean {
    border-color: var(--term-green, #4FE855);
    color: var(--term-green, #4FE855);
  }
  .chip-staged { border-color: var(--term-green, #4FE855); color: var(--term-green, #4FE855); }
  .chip-modified { border-color: var(--amber-bright); color: var(--amber-bright); }
  .chip-untracked { border-color: var(--amber-faint); color: var(--amber-faint); }

  .strip {
    min-height: 28px;
    padding: 4px 14px;
    border-bottom: 1px solid var(--border-subtle);
    box-shadow: var(--depth-edge-light);
    display: flex; align-items: center; gap: 10px;
    background: linear-gradient(to bottom, rgba(212, 137, 10, 0.05), transparent);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    flex-wrap: wrap;
  }
  .strip-label { color: var(--amber-bright); font-weight: 700; }
  .strip-empty { color: var(--amber-faint); font-style: italic; letter-spacing: 0.04em; }

  .log {
    flex: 1;
    display: flex; flex-direction: column;
    min-height: 0;
    border-bottom: 1px solid var(--border-subtle);
  }
  .log-header {
    padding: var(--section-header-padding, 8px 16px);
    color: var(--amber-warm);
    font-size: var(--section-header-size, 11px);
    font-weight: 700;
    letter-spacing: var(--section-header-spacing, 0.1em);
    box-shadow: var(--depth-edge-light), var(--depth-section-sep);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }
  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: 6px 16px 10px;
    color: var(--amber-warm);
    font-size: var(--text-sm);
    box-shadow: var(--depth-inset);
    line-height: 1.5;
    display: flex;
    flex-direction: column;
  }
  .log-body::-webkit-scrollbar { width: 5px; }
  .log-body::-webkit-scrollbar-thumb { background: var(--amber-faint); }

  .empty {
    color: var(--amber-faint);
    font-style: italic;
    padding: 6px 0;
  }
  .empty.error { color: var(--term-red); font-style: normal; }

  .group-header {
    color: var(--amber-bright);
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    padding: 6px 0 2px;
    border-bottom: 1px dashed var(--border-subtle);
    margin-top: 4px;
  }
  .group-header:first-of-type { margin-top: 0; }

  .row {
    display: grid;
    grid-template-columns: 24px 1fr;
    gap: 12px;
    align-items: baseline;
    padding: 2px 4px;
    background: transparent;
    border: none;
    border-left: 2px solid transparent;
    color: inherit;
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    width: 100%;
    transition: background var(--duration-base) ease-out, border-left-color var(--duration-base) ease-out;
  }
  .row:hover {
    background: rgba(212, 137, 10, 0.06);
    border-left-color: var(--amber-bright);
  }
  .file-status {
    font-weight: 700;
    font-size: var(--text-sm);
    text-align: center;
  }
  .path {
    color: var(--amber-warm);
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .state-panel {
    flex-shrink: 0;
    background: var(--bg-panel);
    max-height: 120px;
    overflow-y: auto;
    border-top: 1px solid var(--border-subtle);
    box-shadow: var(--depth-lift), var(--depth-edge-light);
  }
  .state-header {
    padding: var(--section-header-padding, 8px 16px);
    color: var(--amber-warm);
    font-size: var(--section-header-size, 11px);
    font-weight: 700;
    letter-spacing: var(--section-header-spacing, 0.1em);
    box-shadow: var(--depth-edge-light);
    border-bottom: 1px solid var(--border-subtle);
  }
  .state-body {
    padding: 10px 16px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .commit-row {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .commit-hash {
    color: var(--amber-bright);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    font-size: var(--text-xs);
  }
  .commit-subject {
    color: var(--amber-warm);
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .commit-meta {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
  }
  .commit-sep { margin: 0 6px; }
</style>
