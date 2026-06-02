export interface Keybinding {
  key: string;
  description: string;
  category: 'navigation' | 'terminal' | 'editor' | 'window';
}

export const keybindings: Keybinding[] = [
  // Navigation
  { key: 'Ctrl+K', description: 'Command palette', category: 'navigation' },
  { key: 'Ctrl+Shift+P', description: 'Slash launcher (Rift / Claude commands)', category: 'navigation' },
  { key: 'Ctrl+B', description: 'Toggle cockpit panel', category: 'navigation' },
  { key: 'Ctrl+?', description: 'Keyboard shortcuts', category: 'navigation' },

  // Terminal
  { key: 'Ctrl+Shift+F', description: 'Search terminal', category: 'terminal' },
  { key: 'Ctrl+Shift+K', description: 'Mark session moment', category: 'terminal' },
  { key: 'Ctrl+=', description: 'Zoom in', category: 'terminal' },
  { key: 'Ctrl+-', description: 'Zoom out', category: 'terminal' },
  { key: 'Ctrl+0', description: 'Reset zoom', category: 'terminal' },
  { key: 'Ctrl+Shift+C', description: 'Copy selection', category: 'terminal' },
  { key: 'Ctrl+Shift+V', description: 'Paste', category: 'terminal' },

  // Editor (Viewer)
  { key: 'Ctrl+E', description: 'Toggle edit mode', category: 'editor' },
  { key: 'Ctrl+S', description: 'Save file', category: 'editor' },

  // Bus Tail
  { key: 'Ctrl+D', description: 'Bookmark focused event (Bus Tail)', category: 'navigation' },
  { key: 'Ctrl+Shift+N', description: 'Annotate focused event (Bus Tail)', category: 'navigation' },

  // Window
  { key: 'Escape', description: 'Close overlay / dismiss', category: 'window' },
  { key: '/', description: 'Focus search (in Index)', category: 'window' },
];

export const categoryLabels: Record<Keybinding['category'], string> = {
  navigation: 'NAVIGATION',
  terminal: 'TERMINAL',
  editor: 'EDITOR',
  window: 'GENERAL',
};

export const categoryOrder: Keybinding['category'][] = [
  'navigation', 'terminal', 'editor', 'window',
];
