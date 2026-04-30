<script lang="ts">
  // §10.2 — two-row status line. Color-block segments, dark text on
  // category-tinted backgrounds. All values bold. Segment data sources:
  //   * dir / git / repo    → live via translators/status.rs (Category::Status, 5s poll)
  //   * skill               → live via aegis.session.skill_loaded (Phase 7.4 / Aegis)
  //   * effort              → live via aegis.session.effort (D-016 closed; producer
  //                            in the private rift-aegis crate, feature-gated; on
  //                            public-CI builds without the `aegis` feature, no
  //                            envelope is published and the segment stays '—')
  //   * ctx / session / week / model → em-dash placeholder; D-012 upstream-blocked on
  //                            Claude Code usage hook
  //
  // Phase 8.7g.2 — colors split by category instead of all-amber:
  //   GREEN  — env locale (DIR, GIT, REPO)
  //   AMBER  — model+aegis state (MODEL, CTX, SKILL, EFFORT)
  //   PURPLE — session/clock (SESSION)
  //   BLUE   — usage budget (SESSION USE, WEEK)

  interface Props {
    dir?: string;
    model?: string;
    ctx?: string;
    session?: string;
    skill?: string;
    effort?: string;
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
    effort = '—',
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
    <div class="seg session">
      <span class="label">SESSION</span><span class="value">{session}</span>
    </div>
    <div class="seg skill">
      <span class="label">SKILL</span><span class="value">{skill}</span>
    </div>
    <div class="seg effort">
      <span class="label">EFFORT</span><span class="value">{effort}</span>
    </div>
    <div class="seg spacer"></div>
  </div>
  <div class="row">
    <div class="seg git">
      <span class="label">GIT</span><span class="value">{git}</span>
    </div>
    <div class="seg repo">
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

  /* Phase 8.7g.2 — segment colors split by metric category instead of
     all-amber. Within each family the bright/mid/dim shades stay
     distinguishable from one another while reading as the same group. */

  /* GREEN family — env locale */
  .dir    { background: var(--status-green-bright); }
  .git    { background: var(--status-green-mid); }
  .repo   { background: var(--status-green-dim); }

  /* AMBER family — model + Aegis state */
  .model  { background: var(--amber-warm); }
  .ctx    { background: var(--amber-bright); }
  .skill  { background: var(--amber-primary); }
  .effort { background: var(--amber-dim); }

  /* PURPLE — session clock */
  .session { background: var(--status-time); }

  /* BLUE family — usage budget */
  .usage  { background: var(--status-blue-bright); }
  .week   { background: var(--status-blue-dim); }
</style>
