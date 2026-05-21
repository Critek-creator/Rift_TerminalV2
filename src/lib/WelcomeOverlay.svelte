<script lang="ts">
  interface Props {
    ondismiss: () => void;
  }

  let { ondismiss }: Props = $props();

  let step = $state(0);
  const totalSteps = 5;

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
          <p>Rift gets richer when connected to other tools. Tabs light up automatically when integrations are detected:</p>
          <div class="tab-list">
            <div class="tab-item"><span class="tab-icon">◉</span> <strong>Aegis</strong> — appears when the Aegis command center is active</div>
            <div class="tab-item"><span class="tab-icon">◊</span> <strong>Agents</strong> — shows agent activity and Sentinel violations</div>
            <div class="tab-item"><span class="tab-icon">◈</span> <strong>Index</strong> — Abyssal Index vault browser</div>
            <div class="tab-item"><span class="tab-icon">⬡</span> <strong>MCP</strong> — MCP tool dashboard and metrics</div>
          </div>
          <p>No integration? No empty tab. The cockpit only shows what's actually there.</p>
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
    gap: 10px;
    padding: 14px 18px;
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
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.14em;
    color: var(--amber-bright, #FFC840);
  }

  .skip-btn {
    background: transparent;
    border: 1px solid var(--amber-faint, #A87830);
    color: var(--amber-faint, #A87830);
    font-family: inherit;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    padding: 2px 10px;
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
    padding: 20px 24px;
    min-height: 260px;
  }

  .step-title {
    margin: 0 0 12px;
    font-size: 16px;
    font-weight: 700;
    color: var(--amber-bright, #FFC840);
    letter-spacing: 0.02em;
  }

  .step p {
    color: var(--term-white, #E8E4D8);
    font-size: 13px;
    line-height: 1.6;
    margin: 0 0 10px;
  }
  .step p strong {
    color: var(--amber-warm, #E8B840);
  }

  .hint {
    color: var(--amber-faint, #A87830);
    font-size: 10px;
    font-style: italic;
    margin-top: 14px;
  }

  .lane-grid {
    display: grid;
    grid-template-columns: 10px 1fr 10px 1fr;
    gap: 6px 10px;
    align-items: center;
    padding: 8px 0;
  }
  .lane-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .lane-label {
    color: var(--term-white, #E8E4D8);
    font-size: 11px;
  }

  .tag-example {
    display: inline-block;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    padding: 1px 6px;
    border: 1px solid var(--amber-faint, #A87830);
    border-radius: 2px;
    color: var(--amber-warm, #E8B840);
    margin: 0 2px;
  }

  .tab-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 0;
  }
  .tab-item {
    color: var(--term-white, #E8E4D8);
    font-size: 12px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .tab-icon {
    font-size: 13px;
    width: 18px;
    text-align: center;
    color: var(--amber-bright, #FFC840);
  }

  .tip-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 0;
  }
  .tip {
    color: var(--term-white, #E8E4D8);
    font-size: 12px;
    display: flex;
    align-items: center;
    gap: 10px;
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
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
  }

  .welcome-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 18px;
    border-top: 1px solid var(--border-subtle, rgba(255, 168, 38, 0.15));
    background: var(--bg-elevated, #1e1a14);
  }

  .step-dots {
    display: flex;
    gap: 6px;
  }
  .dot {
    width: 8px;
    height: 8px;
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
    gap: 8px;
  }
  .nav-btn {
    background: transparent;
    border: 1px solid var(--amber-faint, #A87830);
    color: var(--amber-faint, #A87830);
    font-family: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.1em;
    padding: 5px 16px;
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
