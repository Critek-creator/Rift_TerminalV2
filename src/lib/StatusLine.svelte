<script lang="ts">
  // §10.2 — two-row status line. Color-block segments with dark text on
  // brand-amber backgrounds. All values bold. Phase 2 ships the visual
  // chassis; live values plumb in later phases:
  //   * dir / repo / git    → Phase 5 (lightweight Rust helper)
  //   * ctx / session / use → still pending — needs Claude Code hook with usage payload (Phase 7.4b candidate)
  //   * skill               → live via aegis.session.skill_loaded (Phase 7.4)

  interface Props {
    dir?: string;
    model?: string;
    ctx?: string;
    session?: string;
    skill?: string;
    git?: string;
    repo?: string;
    sessionUse?: string;
    week?: string;
  }

  let {
    dir = '—',
    model = '—',
    ctx = '—',
    session = '—',
    skill = '—',
    git = '—',
    repo = '—',
    sessionUse = '—',
    week = '—',
  }: Props = $props();
</script>

<footer class="statusline">
  <div class="row">
    <div class="seg dir">
      <span class="label">DIR</span><span class="value">{dir}</span>
    </div>
    <div class="seg model">
      <span class="label">MODEL</span><span class="value">{model}</span>
    </div>
    <div class="seg ctx">
      <span class="label">CTX</span><span class="value">{ctx}</span>
    </div>
    <div class="seg time">
      <span class="label">SESSION</span><span class="value">{session}</span>
    </div>
    <div class="seg skill">
      <span class="label">SKILL</span><span class="value">{skill}</span>
    </div>
    <div class="seg spacer"></div>
  </div>
  <div class="row">
    <div class="seg git">
      <span class="label">GIT</span><span class="value">{git}</span>
    </div>
    <div class="seg gitdir">
      <span class="label">REPO</span><span class="value">{repo}</span>
    </div>
    <div class="seg usage">
      <span class="label">SESSION USE</span><span class="value">{sessionUse}</span>
    </div>
    <div class="seg week">
      <span class="label">WEEK</span><span class="value">{week}</span>
    </div>
    <div class="seg spacer"></div>
  </div>
</footer>

<style>
  .statusline {
    flex-shrink: 0;
    background: var(--bg-surface);
    border-top: 1px solid var(--border-subtle);
    font-size: 11px;
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
    gap: 6px;
    padding: 0 10px;
    border-right: 1px solid var(--border-subtle);
    white-space: nowrap;
  }
  .seg:last-child { border-right: none; }

  .label {
    color: rgba(0, 0, 0, 0.65);
    font-weight: 700;
    font-size: 9px;
    letter-spacing: 0.1em;
  }
  .value {
    color: rgba(0, 0, 0, 0.92);
    font-weight: 700;
  }
  .spacer {
    flex: 1;
    border-right: none;
    background: transparent !important;
  }

  /* Segment color blocks — brand palette */
  .dir    { background: var(--amber-primary); }
  .model  { background: var(--amber-warm); }
  .ctx    { background: var(--amber-bright); }
  .time   { background: var(--amber-dim); }
  .skill  { background: var(--amber-warm); }
  .git    { background: var(--term-green); }
  .gitdir { background: var(--amber-warm); }
  .usage  { background: var(--amber-primary); }
  .week   { background: var(--amber-bright); }
</style>
