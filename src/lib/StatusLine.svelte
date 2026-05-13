<script lang="ts">
  // §10.2 — two-row status line. Color-block segments, dark text on
  // category-tinted backgrounds. All values bold. Segment data sources:
  //   * dir / git / repo    → live via translators/status.rs (Category::Status, 5s poll)
  //   * skill               → live via aegis.session.skill_loaded (Phase 7.4 / Aegis)
  //   * effort              → live via aegis.session.effort (D-016 closed; producer
  //                            in the private rift-aegis crate, feature-gated; on
  //                            public-CI builds without the `aegis` feature, no
  //                            envelope is published and the segment stays '—')
  //   * model / ctx / sessionUse / week → D-012 blocked (upstream Claude Code usage
  //                            hook not yet available); segments hidden until unblocked
  //
  // Phase 8.7g.2 — colors split by category instead of all-amber:
  //   GREEN  — env locale (DIR, GIT, REPO)
  //   AMBER  — model+aegis state (SKILL, EFFORT)
  //   PURPLE — session/clock (SESSION)

  interface Props {
    dir?: string;
    session?: string;
    skill?: string;
    effort?: string;
    git?: string;
    repo?: string;
  }

  let {
    dir = '—',
    session = '—',
    skill = '—',
    effort = '—',
    git = '—',
    repo = '—',
  }: Props = $props();
</script>

<footer class="statusline">
  <div class="row">
    <div class="seg dir">
      <span class="label">DIR</span><span class="value">{dir}</span>
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
     distinguishable from one another while reading as the same group.
     model / ctx / sessionUse / week removed (D-012 blocked — hidden until
     upstream Claude Code usage hook is available). */

  /* GREEN family — env locale */
  .dir    { background: var(--status-green-bright); }
  .git    { background: var(--status-green-mid); }
  .repo   { background: var(--status-green-dim); }

  /* AMBER family — Aegis state */
  .skill  { background: var(--amber-primary); }
  .effort { background: var(--amber-dim); }

  /* PURPLE — session clock */
  .session { background: var(--status-time); }
</style>
