import { invoke } from '@tauri-apps/api/core';
import type { RiftConfig } from './riftConfig';

const TERM_DEFAULT_FONT_SIZE = 13;
const TERM_MIN_FONT_SIZE = 8;
const TERM_MAX_FONT_SIZE = 48;
const TERM_DEFAULT_LINE_HEIGHT = 1.55;
const TERM_DEFAULT_SCROLLBACK = 1000;

export interface TerminalSettings {
  fontSize: number;
  lineHeight: number;
  scrollback: number;
  lanesEnabled: boolean;
  colorPalette: string;
}

const FALLBACK: TerminalSettings = {
  fontSize: TERM_DEFAULT_FONT_SIZE,
  lineHeight: TERM_DEFAULT_LINE_HEIGHT,
  scrollback: TERM_DEFAULT_SCROLLBACK,
  lanesEnabled: true,
  colorPalette: 'amber',
};

let cached: TerminalSettings | null = null;

export async function getTerminalSettings(): Promise<TerminalSettings> {
  if (cached) return cached;
  try {
    const cfg = await invoke<RiftConfig>('config_get');
    const t = cfg?.terminal ?? null;
    if (!t) {
      cached = { ...FALLBACK };
      return cached;
    }
    cached = {
      fontSize: Math.max(TERM_MIN_FONT_SIZE, Math.min(TERM_MAX_FONT_SIZE, t.font_size)),
      lineHeight: Math.max(1.0, Math.min(2.5, t.line_height)),
      scrollback: Math.max(100, Math.min(100000, t.scrollback)),
      lanesEnabled: t.lanes_enabled,
      colorPalette: t.color_palette ?? 'amber',
    };
    return cached;
  } catch {
    cached = { ...FALLBACK };
    return cached;
  }
}

export function invalidateTerminalSettingsCache(): void {
  cached = null;
}
