import type { Terminal as XTerm, IDecoration } from '@xterm/xterm';

const TINT_COLORS: Record<string, string> = {
  CLAUDE: 'rgba(108, 182, 255, 0.06)',
  AGENT:  'rgba(197, 143, 255, 0.06)',
  HOOK:   'rgba(111, 224, 224, 0.06)',
  AEGIS:  'rgba(255, 168, 38, 0.06)',
  ERR:    'rgba(255, 72, 72, 0.08)',
  WARN:   'rgba(255, 200, 64, 0.05)',
};

const MAX_DECORATIONS = 500;

export class LaneTintManager {
  private term: XTerm;
  private currentLane = 'SYS';
  private segmentStartRow = 0;
  private decorations: IDecoration[] = [];

  constructor(term: XTerm) {
    this.term = term;
    this.segmentStartRow = this.absRow();
  }

  private absRow(): number {
    const buf = this.term.buffer.active;
    return buf.baseY + buf.cursorY;
  }

  onLaneChanged(lane: string): void {
    const row = this.absRow();
    const color = TINT_COLORS[this.currentLane];
    if (color && row > this.segmentStartRow) {
      this.tintRange(this.segmentStartRow, row, color);
    }
    this.currentLane = lane;
    this.segmentStartRow = row;
  }

  private tintRange(start: number, end: number, color: string): void {
    const cur = this.absRow();
    for (let r = start; r < end; r++) {
      const marker = this.term.registerMarker(r - cur);
      if (!marker) continue;
      const deco = this.term.registerDecoration({ marker });
      if (!deco) continue;
      deco.onRender(el => {
        el.style.backgroundColor = color;
        el.style.width = '100%';
        el.style.height = '100%';
        el.style.pointerEvents = 'none';
      });
      this.decorations.push(deco);
    }
    while (this.decorations.length > MAX_DECORATIONS) {
      this.decorations.shift()?.dispose();
    }
  }

  dispose(): void {
    for (const d of this.decorations) d.dispose();
    this.decorations = [];
  }
}
