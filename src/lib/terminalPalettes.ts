import type { ITheme } from '@xterm/xterm';

export interface TerminalPalette {
  id: string;
  label: string;
  description: string;
  theme: ITheme;
}

export const CUSTOM_PALETTE_KEYS = [
  'background', 'foreground', 'cursor', 'cursorAccent', 'selectionBackground',
  'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
  'brightBlack', 'brightRed', 'brightGreen', 'brightYellow',
  'brightBlue', 'brightMagenta', 'brightCyan', 'brightWhite',
] as const;

export type PaletteColorKey = (typeof CUSTOM_PALETTE_KEYS)[number];

export const PALETTE_KEY_LABELS: Record<PaletteColorKey, string> = {
  background: 'Background',
  foreground: 'Foreground',
  cursor: 'Cursor',
  cursorAccent: 'Cursor Accent',
  selectionBackground: 'Selection',
  black: 'Black',
  red: 'Red',
  green: 'Green',
  yellow: 'Yellow',
  blue: 'Blue',
  magenta: 'Magenta',
  cyan: 'Cyan',
  white: 'White',
  brightBlack: 'Bright Black',
  brightRed: 'Bright Red',
  brightGreen: 'Bright Green',
  brightYellow: 'Bright Yellow',
  brightBlue: 'Bright Blue',
  brightMagenta: 'Bright Magenta',
  brightCyan: 'Bright Cyan',
  brightWhite: 'Bright White',
};

export const PALETTES: TerminalPalette[] = [
  {
    id: 'amber',
    label: 'Amber Classic',
    description: 'Warm amber on vantablack — the original CRT',
    theme: {
      background: '#000000',
      foreground: '#FFA826',
      cursor: '#FFC840',
      cursorAccent: '#000000',
      selectionBackground: 'rgba(255, 168, 38, 0.30)',
      black: '#3A3530',
      red: '#FF4848',
      green: '#4FE855',
      yellow: '#FFC840',
      blue: '#6CB6FF',
      magenta: '#C58FFF',
      cyan: '#6FE0E0',
      white: '#E8E4D8',
      brightBlack: '#C49A50',
      brightRed: '#FF6868',
      brightGreen: '#7FFA85',
      brightYellow: '#FFD968',
      brightBlue: '#9CCEFF',
      brightMagenta: '#DAB1FF',
      brightCyan: '#9FF0F0',
      brightWhite: '#FFFAEC',
    },
  },
  {
    id: 'vivid',
    label: 'Vivid',
    description: 'High-contrast bright colors — maximum legibility',
    theme: {
      background: '#0A0A08',
      foreground: '#FFB840',
      cursor: '#FFD060',
      cursorAccent: '#0A0A08',
      selectionBackground: 'rgba(255, 184, 64, 0.35)',
      black: '#2A2520',
      red: '#FF5555',
      green: '#50FA7B',
      yellow: '#F1FA8C',
      blue: '#82AAFF',
      magenta: '#FF79C6',
      cyan: '#8BE9FD',
      white: '#F8F8F2',
      brightBlack: '#D4A050',
      brightRed: '#FF6E6E',
      brightGreen: '#70FF94',
      brightYellow: '#FFFFA5',
      brightBlue: '#A4CCFF',
      brightMagenta: '#FF92D0',
      brightCyan: '#A4F4FF',
      brightWhite: '#FFFFFF',
    },
  },
  {
    id: 'muted',
    label: 'Muted',
    description: 'Softer, desaturated tones — easy on the eyes',
    theme: {
      background: '#0C0C0A',
      foreground: '#C8963A',
      cursor: '#D4A850',
      cursorAccent: '#0C0C0A',
      selectionBackground: 'rgba(200, 150, 58, 0.25)',
      black: '#3A3530',
      red: '#CC5050',
      green: '#5CB868',
      yellow: '#C8A848',
      blue: '#6899CC',
      magenta: '#9978B8',
      cyan: '#5AADAD',
      white: '#B8B4A8',
      brightBlack: '#9A7840',
      brightRed: '#D86868',
      brightGreen: '#78C880',
      brightYellow: '#D4B860',
      brightBlue: '#88B4DD',
      brightMagenta: '#B098CC',
      brightCyan: '#78C4C4',
      brightWhite: '#D4D0C4',
    },
  },
  {
    id: 'phosphor',
    label: 'Phosphor',
    description: 'Green phosphor CRT — retro terminal aesthetic',
    theme: {
      background: '#060806',
      foreground: '#33FF33',
      cursor: '#66FF66',
      cursorAccent: '#060806',
      selectionBackground: 'rgba(51, 255, 51, 0.25)',
      black: '#1A2A1A',
      red: '#FF4848',
      green: '#33FF33',
      yellow: '#CCFF33',
      blue: '#33CCFF',
      magenta: '#CC66FF',
      cyan: '#33FFCC',
      white: '#CCFFCC',
      brightBlack: '#448844',
      brightRed: '#FF6868',
      brightGreen: '#66FF66',
      brightYellow: '#DDFF66',
      brightBlue: '#66DDFF',
      brightMagenta: '#DD88FF',
      brightCyan: '#66FFDD',
      brightWhite: '#EEFFEE',
    },
  },
  {
    id: 'midnight',
    label: 'Midnight',
    description: 'Cool blue-silver on deep navy — calm night coding',
    theme: {
      background: '#0B0E14',
      foreground: '#B8C4D8',
      cursor: '#6CB6FF',
      cursorAccent: '#0B0E14',
      selectionBackground: 'rgba(108, 182, 255, 0.22)',
      black: '#1A1F2E',
      red: '#F07178',
      green: '#AAD94C',
      yellow: '#E6B450',
      blue: '#59C2FF',
      magenta: '#D2A6FF',
      cyan: '#73B8FF',
      white: '#C7D3E8',
      brightBlack: '#4A5568',
      brightRed: '#FF8F9A',
      brightGreen: '#C4ED72',
      brightYellow: '#FFCF6E',
      brightBlue: '#7DD4FF',
      brightMagenta: '#E4C4FF',
      brightCyan: '#95D0FF',
      brightWhite: '#E8EEF8',
    },
  },
  {
    id: 'solarized',
    label: 'Solarized Dark',
    description: 'Ethan Schoonover\'s precision-balanced palette',
    theme: {
      background: '#002B36',
      foreground: '#839496',
      cursor: '#93A1A1',
      cursorAccent: '#002B36',
      selectionBackground: 'rgba(147, 161, 161, 0.20)',
      black: '#073642',
      red: '#DC322F',
      green: '#859900',
      yellow: '#B58900',
      blue: '#268BD2',
      magenta: '#D33682',
      cyan: '#2AA198',
      white: '#EEE8D5',
      brightBlack: '#586E75',
      brightRed: '#CB4B16',
      brightGreen: '#586E75',
      brightYellow: '#657B83',
      brightBlue: '#839496',
      brightMagenta: '#6C71C4',
      brightCyan: '#93A1A1',
      brightWhite: '#FDF6E3',
    },
  },
  {
    id: 'dracula',
    label: 'Dracula',
    description: 'Purple-accent dark theme — rich and inviting',
    theme: {
      background: '#282A36',
      foreground: '#F8F8F2',
      cursor: '#F8F8F2',
      cursorAccent: '#282A36',
      selectionBackground: 'rgba(68, 71, 90, 0.60)',
      black: '#21222C',
      red: '#FF5555',
      green: '#50FA7B',
      yellow: '#F1FA8C',
      blue: '#BD93F9',
      magenta: '#FF79C6',
      cyan: '#8BE9FD',
      white: '#F8F8F2',
      brightBlack: '#6272A4',
      brightRed: '#FF6E6E',
      brightGreen: '#69FF94',
      brightYellow: '#FFFFA5',
      brightBlue: '#D6ACFF',
      brightMagenta: '#FF92DF',
      brightCyan: '#A4FFFF',
      brightWhite: '#FFFFFF',
    },
  },
  {
    id: 'high-contrast',
    label: 'High Contrast',
    description: 'WCAG AAA — stark white on black, vivid primaries',
    theme: {
      background: '#000000',
      foreground: '#FFFFFF',
      cursor: '#FFFFFF',
      cursorAccent: '#000000',
      selectionBackground: 'rgba(255, 255, 255, 0.30)',
      black: '#000000',
      red: '#FF0000',
      green: '#00FF00',
      yellow: '#FFFF00',
      blue: '#0080FF',
      magenta: '#FF00FF',
      cyan: '#00FFFF',
      white: '#FFFFFF',
      brightBlack: '#808080',
      brightRed: '#FF5555',
      brightGreen: '#55FF55',
      brightYellow: '#FFFF55',
      brightBlue: '#5599FF',
      brightMagenta: '#FF55FF',
      brightCyan: '#55FFFF',
      brightWhite: '#FFFFFF',
    },
  },
];

export function getPalette(id: string): TerminalPalette {
  return PALETTES.find(p => p.id === id) ?? PALETTES[0];
}

export function getDefaultCustomColors(): Record<string, string> {
  const base = PALETTES[0].theme;
  const result: Record<string, string> = {};
  for (const key of CUSTOM_PALETTE_KEYS) {
    const val = base[key as keyof ITheme];
    if (typeof val === 'string') {
      result[key] = val;
    }
  }
  return result;
}

export function buildCustomTheme(overrides: Record<string, string>): ITheme {
  const base = getDefaultCustomColors();
  const merged = { ...base, ...overrides };
  const theme: ITheme = {};
  for (const key of CUSTOM_PALETTE_KEYS) {
    if (merged[key]) {
      (theme as Record<string, string>)[key] = merged[key];
    }
  }
  return theme;
}

export function resolveTheme(paletteId: string, customColors?: Record<string, string>): ITheme {
  if (paletteId === 'custom') {
    return buildCustomTheme(customColors ?? {});
  }
  return getPalette(paletteId).theme;
}
