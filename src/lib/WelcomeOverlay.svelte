<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';

  interface IntegrationDetail {
    installed: boolean;
    enabled: boolean;
    path: string;
  }

  interface IntegrationStatus {
    claude_dir_exists: boolean;
    node_available: boolean;
    node_version: string | null;
    aegis: IntegrationDetail;
    index: IntegrationDetail;
  }

  interface Props {
    ondismiss: () => void;
  }

  let { ondismiss }: Props = $props();

  let step = $state(0);
  const totalSteps = 5;

  let integrations = $state<IntegrationStatus | null>(null);

  $effect(() => {
    if (step === 3 && !integrations) {
      invoke<IntegrationStatus>('integration_detect').then((s) => {
        integrations = s;
      });
    }
  });

  function statusIcon(detail: IntegrationDetail | undefined): string {
    if (!detail) return '…';
    if (detail.installed && detail.enabled) return '✅';
    if (detail.installed) return '⏸';
    return '⚠';
  }

  function statusLabel(detail: IntegrationDetail | undefined): string {
    if (!detail) return 'Detecting…';
    if (detail.installed && detail.enabled) return 'Installed & active';
    if (detail.installed) return 'Installed (disabled)';
    return 'Not installed';
  }

  function next() {
    if (step < totalSteps - 1) step++;
    else ondismiss();
  }

  function prev() {
    if (step > 0) step--;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') ondismiss();
    else if (e.key === 'ArrowRight' || e.key === 'Enter') next();
    else if (e.key === 'ArrowLeft') prev();
  }
</script>

<div
  class="welcome-backdrop"
  role="presentation"
  onclick={ondismiss}
  onkeydown={onKeydown}
>
  <div
    class="welcome-panel"
    role="dialog"
    aria-modal="true"
    aria-label="Welcome to Rift"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={onKeydown}
  >
    <header class="welcome-header">
      <span class="header-icon">◈</span>
      <span class="header-title">WELCOME TO RIFT</span>
      <button type="button" class="skip-btn" onclick={ondismiss}>SKIP</button>
    </header>

    <div class="welcome-body">
      {#if step === 0}
        <div class="step">
          <h2 class="step-title">One Product, Two Surfaces</h2>
          <p>Rift is a <strong>terminal emulator</strong> and a <strong>cockpit</strong> in one window. The terminal is where you work. The cockpit watches what happens and shows you the patterns.</p>
          <p>Every command you run, every file that changes, every hook that fires — Rift sees it all and organizes it into notification tabs on the right side of your tab bar.</p>
          <div class="hint">Built by Abyssal Arts with Rust + Tauri + Svelte</div>
        </div>
      {:else if step === 1}
        <div class="step">
          <h2 class="step-title">The Terminal</h2>
          <p>Your terminal output is <strong>color-coded by lane</strong>. Each lane represents a different source:</p>
          <div class="lane-grid">
            <span class="lane-dot" style="background: var(--term-blue)"></span><span class="lane-label">Claude voice</span>
            <span class="lane-dot" style="background: var(--term-purple)"></span><span class="lane-label">Agent output</span>
            <span class="lane-dot" style="background: var(--term-cyan)"></span><span class="lane-label">Hook events</span>
            <span class="lane-dot" style="background: var(--amber-primary)"></span><span class="lane-label">Aegis</span>
            <span class="lane-dot" style="background: var(--term-green)"></span><span class="lane-label">Success</span>
            <span class="lane-dot" style="background: var(--term-red)"></span><span class="lane-label">Errors</span>
          </div>
          <p>Tags like <span class="tag-example">CLAUDE</span> <span class="tag-example">HOOK</span> <span class="tag-example">ERR</span> mark the source at a glance.</p>
        </div>
      {:else if step === 2}
        <div class="step">
          <h2 class="step-title">The Cockpit</h2>
          <p>The <strong>notification tabs</strong> on the right side of the tab bar are your cockpit. Each tab watches a different subsystem:</p>
          <div class="tab-list">
            <div class="tab-item"><span class="tab-icon">⚡</span> <strong>Errors</strong> — aggregated errors and warnings</div>
            <div class="tab-item"><span class="tab-icon">⚓</span> <strong>Hooks</strong> — Claude Code hook activity</div>
            <div class="tab-item"><span class="tab-icon">⌘</span> <strong>Commands</strong> — command history and exit codes</div>
            <div class="tab-item"><span class="tab-icon">⊞</span> <strong>Files</strong> — filesystem activity tree with heatmap</div>
            <div class="tab-item"><span class="tab-icon">⎇</span> <strong>Git</strong> — repository state changes</div>
          </div>
          <p>Click a tab to view it. Drag it off the strip to promote it to a side pane.</p>
        </div>
      {:else if step === 3}
        <div class="step">
          <h2 class="step-title">Integrations</h2>
          <p>Rift gets richer with optional integrations. Tabs light up automatically when detected.</p>
          <div class="integration-list">
            <div class="integration-card">
              <span class="integration-status">{statusIcon(integrations?.aegis)}</span>
              <div class="integration-info">
                <strong>Aegis</strong> — agent observability, session monitoring, maintenance automation
                <div class="integration-detail">{statusLabel(integrations?.aegis)}</div>
              </div>
            </div>
            <div class="integration-card">
              <span class="integration-status">{statusIcon(integrations?.index)}</span>
              <div class="integration-info">
                <strong>Abyssal Index</strong> — vault browser, semantic enrichment, knowledge cockpit
                <div class="integration-detail">{statusLabel(integrations?.index)}</div>
              </div>
            </div>
            <div class="integration-card">
              <span class="integration-status">ℹ</span>
              <div class="integration-info">
                <strong>MCP Tools</strong> — 20 tools for programmatic access
                <div class="integration-detail">Auto-detected when MCP server connects</div>
              </div>
            </div>
          </div>
          {#if integrations && !integrations.claude_dir_exists}
            <div class="integration-warning">Claude Code not detected. Install Claude Code first, then manage integrations in Settings.</div>
          {:else if integrations && (!integrations.aegis.installed || !integrations.index.installed)}
            <p>Manage integrations anytime in <strong>Settings &gt; Integrations</strong>.</p>
          {/if}
        </div>
      {:else if step === 4}
        <div class="step">
          <h2 class="step-title">Get Started</h2>
          <p>You're ready. A few tips:</p>
          <div class="tip-list">
            <div class="tip"><kbd>Ctrl+?</kbd> Keyboard shortcuts</div>
            <div class="tip"><kbd>Ctrl+K</kbd> Command palette</div>
            <div class="tip"><kbd>Ctrl+B</kbd> Toggle cockpit panel</div>
            <div class="tip">Right-click a notification tab to show/hide it</div>
          </div>
          <p>You can reopen this guide anytime from <strong>Settings</strong>.</p>
          <div class="hint">This is a beta — report issues on GitHub. Support development on Patreon.</div>
        </div>
      {/if}
    </div>

    <footer class="welcome-footer">
      <div class="step-dots">
        {#each Array(totalSteps) as _, i}
          <button
            type="button"
            class="dot"
            class:active={i === step}
            aria-label="Go to step {i + 1}"
            onclick={() => (step = i)}
          ></button>
        {/each}
      </div>
      <div class="step-nav">
        {#if step > 0}
          <button type="button" class="nav-btn" onclick={prev}>BACK</button>
        {/if}
        <button type="button" class="nav-btn primary" onclick={next}>
          {step === totalSteps - 1 ? 'START' : 'NEXT'}
        </button>
      </div>
    </footer>
  </div>
</div>

<style>
  .welcome-backdrop {
    position: fixed;
    inset: 0;
    z-index: 200;
    background: rgba(0, 0, 0, 0.65);
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .welcome-panel {
    width: min(560px, 90vw);
    max-height: 80vh;
    background: var(--bg-surface, #1a1814);
    border: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    border-radius: var(--radius-md, 6px);
    box-shadow: 0 12px 48px rgba(0, 0, 0, 0.8);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .welcome-header {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-14) var(--space-lg);
    border-bottom: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    background: var(--bg-elevated, #1e1a14);
  }

  .header-icon {
    font-size: 16px;
    color: var(--amber-bright, #FFC840);
    text-shadow: var(--glow-amber-faint);
  }

  .header-title {
    flex: 1;
    font-size: var(--text-sm);
    font-weight: 700;
    letter-spacing: 0.14em;
    color: var(--amber-bright, #FFC840);
  }

  .skip-btn {
    background: transparent;
    border: 1px solid var(--amber-faint, #A87830);
    color: var(--amber-faint, #A87830);
    font-family: inherit;
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    padding: 2px var(--space-md);
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s;
  }
  .skip-btn:hover {
    color: var(--amber-bright, #FFC840);
    border-color: var(--amber-bright, #FFC840);
  }

  .welcome-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-xl) var(--space-24);
    min-height: 260px;
  }

  .step-title {
    margin: 0 0 var(--space-12);
    font-size: 16px;
    font-weight: 700;
    color: var(--amber-bright, #FFC840);
    letter-spacing: 0.02em;
  }

  .step p {
    color: var(--term-white, #E8E4D8);
    font-size: var(--text-md);
    line-height: 1.6;
    margin: 0 0 var(--space-md);
  }
  .step p strong {
    color: var(--amber-warm, #E8B840);
  }

  .hint {
    color: var(--amber-faint, #A87830);
    font-size: var(--text-xs);
    font-style: italic;
    margin-top: var(--space-14);
  }

  .lane-grid {
    display: grid;
    grid-template-columns: 10px 1fr 10px 1fr;
    gap: var(--space-sm) var(--space-md);
    align-items: center;
    padding: var(--space-8) 0;
  }
  .lane-dot {
    width: var(--space-8);
    height: var(--space-8);
    border-radius: 50%;
  }
  .lane-label {
    color: var(--term-white, #E8E4D8);
    font-size: var(--text-sm);
  }

  .tag-example {
    display: inline-block;
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    padding: 1px var(--space-sm);
    border: 1px solid var(--amber-faint, #A87830);
    border-radius: 2px;
    color: var(--amber-warm, #E8B840);
    margin: 0 2px;
  }

  .tab-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-sm) 0;
  }
  .tab-item {
    color: var(--term-white, #E8E4D8);
    font-size: var(--text-base);
    display: flex;
    align-items: center;
    gap: var(--space-8);
  }
  .tab-icon {
    font-size: var(--text-md);
    width: 18px;
    text-align: center;
    color: var(--amber-bright, #FFC840);
  }

  .tip-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-sm) 0;
  }
  .tip {
    color: var(--term-white, #E8E4D8);
    font-size: var(--text-base);
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }
  .integration-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-8);
    padding: var(--space-sm) 0;
  }
  .integration-card {
    display: flex;
    align-items: flex-start;
    gap: var(--space-md);
    padding: var(--space-8) var(--space-md);
    border: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    border-radius: var(--radius-sm, 4px);
    background: rgba(255, 168, 38, 0.03);
  }
  .integration-status {
    font-size: var(--text-md);
    flex-shrink: 0;
    width: 20px;
    text-align: center;
    margin-top: 1px;
  }
  .integration-info {
    color: var(--term-white, #E8E4D8);
    font-size: var(--text-sm);
    line-height: 1.5;
  }
  .integration-info strong {
    color: var(--amber-warm, #E8B840);
  }
  .integration-detail {
    font-size: var(--text-xs);
    color: var(--amber-faint, #A87830);
    margin-top: 2px;
  }
  .integration-warning {
    margin-top: var(--space-md);
    padding: var(--space-8) var(--space-md);
    border: 1px solid var(--term-red, #CC3333);
    border-radius: var(--radius-sm, 4px);
    color: var(--term-red, #CC3333);
    font-size: var(--text-xs);
    background: rgba(204, 51, 51, 0.06);
  }

  .tip kbd {
    display: inline-block;
    min-width: 90px;
    text-align: right;
    background: var(--bg-elevated, #1e1a14);
    border: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    border-radius: 3px;
    color: var(--amber-primary, #FFA826);
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    padding: 2px var(--space-8);
  }

  .welcome-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-12) var(--space-lg);
    border-top: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    background: var(--bg-elevated, #1e1a14);
  }

  .step-dots {
    display: flex;
    gap: var(--space-sm);
  }
  .dot {
    width: var(--space-8);
    height: var(--space-8);
    border-radius: 50%;
    background: var(--amber-faint, #A87830);
    border: none;
    cursor: pointer;
    padding: 0;
    transition: background 0.15s, box-shadow 0.15s;
  }
  .dot.active {
    background: var(--amber-bright, #FFC840);
    box-shadow: 0 0 6px rgba(255, 200, 64, 0.5);
  }

  .step-nav {
    display: flex;
    gap: var(--space-8);
  }
  .nav-btn {
    background: transparent;
    border: 1px solid var(--amber-faint, #A87830);
    color: var(--amber-faint, #A87830);
    font-family: inherit;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    padding: 5px var(--space-lg);
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s, background 0.12s;
  }
  .nav-btn:hover {
    color: var(--amber-bright, #FFC840);
    border-color: var(--amber-bright, #FFC840);
  }
  .nav-btn.primary {
    background: rgba(255, 168, 38, 0.12);
    border-color: var(--amber-primary, #FFA826);
    color: var(--amber-primary, #FFA826);
  }
  .nav-btn.primary:hover {
    background: rgba(255, 168, 38, 0.22);
    color: var(--amber-bright, #FFC840);
    border-color: var(--amber-bright, #FFC840);
  }
</style>
