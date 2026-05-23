// laneFormat — emit §10.1 lane tag prefix + ANSI color for a string written
// directly to xterm.js. Used by Rift-owned messages (session-exited,
// pty-failed, etc.) so they read consistently with the rest of the §10.1
// visual system.
//
// Scope (D-018 partial). This helper covers ONLY lines Rift emits itself
// — strings that flow through `term.write()` without first passing through
// the PTY. Live PTY-stream lane classification (CLAUDE / AGENT / HOOK output
// from inside the spawned shell) is tracked separately under DEFERRED.md
// D-018 and needs a translator-driven design beat. See `RIFT_V2_VISION.md`
// §10.1 + §9 for the locked spec.
//
// Colors mirror `src/styles.css` lane tokens. xterm.js renders 24-bit color
// via `\x1b[38;2;R;G;Bm`, so the palette is duplicated here as numeric RGB
// triples — the CSS hex values are NOT readable from xterm's render layer.
// Drift between the two is caught by the integration test that imports both.

/** §10.1 lanes recognized by the formatter. Keep aligned with the
 *  `.lane-*` classes in styles.css and the locked palette in §10.1. */
export type Lane =
  | 'CLAUDE'
  | 'AGENT'
  | 'HOOK'
  | 'AEGIS'
  | 'OK'
  | 'WARN'
  | 'ERR'
  | 'SYS';

interface LaneStyle {
  /** 24-bit RGB foreground. */
  rgb: [number, number, number];
  /** When true, wrap content in italic (`\x1b[3m`). */
  italic?: boolean;
}

/**
 * Lane palette mirroring `styles.css :root` tokens. Update both files
 * together if §10.1 lane colors are tuned.
 *
 *   CLAUDE  → --term-blue     #6CB6FF
 *   AGENT   → --term-purple   #C58FFF
 *   HOOK    → --term-cyan     #6FE0E0
 *   AEGIS   → --amber-primary #FFA826  (approx; exact token in styles.css)
 *   OK      → --term-green    #4FE855
 *   WARN    → --amber-bright  #FFC840
 *   ERR     → --term-red      #FF4848
 *   SYS     → --amber-faint   #A87830  italic
 */
const LANE_STYLES: Record<Lane, LaneStyle> = {
  CLAUDE: { rgb: [108, 182, 255] },
  AGENT: { rgb: [197, 143, 255] },
  HOOK: { rgb: [111, 224, 224] },
  AEGIS: { rgb: [255, 168, 38] },
  OK: { rgb: [79, 232, 85] },
  WARN: { rgb: [255, 200, 64] },
  ERR: { rgb: [255, 72, 72] },
  SYS: { rgb: [168, 120, 48], italic: true },
};

const ANSI_RESET = '\x1b[0m';
const ANSI_ITALIC = '\x1b[3m';

function ansiFg(r: number, g: number, b: number): string {
  return `\x1b[38;2;${r};${g};${b}m`;
}

/**
 * Format `message` with the §10.1 tag prefix + lane color, followed by an
 * ANSI reset. The output is a complete, self-contained span — appending
 * subsequent text via `term.write()` will not inherit the lane color.
 *
 * Shape:  `[LANE] message`
 *  - tag prefix uses the same color as the body (not a separate border;
 *    monospace cells can't render the §10.1 bordered-box style cleanly,
 *    so the bracket form is the terminal-surface equivalent of the
 *    `.tag-*` HTML classes used in NotificationPane / AegisLogTab).
 *  - SYS is italicized per the styles.css `.lane-meta` rule.
 *
 * @param lane The §10.1 lane.
 * @param message The body text (already trimmed of trailing newlines).
 *                Any embedded `\x1b[0m` will close the lane formatting
 *                early — callers shouldn't pre-color their content.
 * @returns The tagged + colored string. No leading or trailing newline
 *          is added — caller controls cursor placement.
 */
function laneFormat(lane: Lane, message: string): string {
  const style = LANE_STYLES[lane];
  const fg = ansiFg(style.rgb[0], style.rgb[1], style.rgb[2]);
  const italic = style.italic ? ANSI_ITALIC : '';
  return `${fg}${italic}[${lane}] ${message}${ANSI_RESET}`;
}

/**
 * Same as [`laneFormat`] but returns plain text (no ANSI codes) when
 * `lanesEnabled` is false. Used by callers that read
 * `RiftConfig.terminal.lanes_enabled` and want to honor the user's
 * opt-out without branching at every emit site.
 */
export function laneFormatGated(lane: Lane, message: string, lanesEnabled: boolean): string {
  if (!lanesEnabled) return message;
  return laneFormat(lane, message);
}
