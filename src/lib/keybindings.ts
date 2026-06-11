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
  { key: 'Ctrl+Shift+M', description: 'Switch model', category: 'navigation' },
  { key: 'Ctrl+Shift+L', description: 'Ensemble compare', category: 'navigation' },
  { key: 'Ctrl+Shift+A', description: 'Open LLM chat', category: 'navigation' },

  // Terminal
  { key: 'Ctrl+T', description: 'New session', category: 'terminal' },
  { key: 'Ctrl+Shift+F', description: 'Search terminal', category: 'terminal' },
  { key: 'Ctrl+Shift+K', description: 'Mark session moment', category: 'terminal' },
  { key: 'Ctrl+=', description: 'Zoom in', category: 'terminal' },
  { key: 'Ctrl+-', description: 'Zoom out', category: 'terminal' },
  { key: 'Ctrl+0', description: 'Reset zoom', category: 'terminal' },
  { key: 'Ctrl+Shift+C', description: 'Copy selection', category: 'terminal' },
  { key: 'Ctrl+Shift+V', description: 'Paste', category: 'terminal' },
  { key: 'Ctrl+Shift+E', description: 'Split pane down', category: 'terminal' },
  { key: 'Ctrl+Shift+D', description: 'Split pane right', category: 'terminal' },
  { key: 'Ctrl+Shift+W', description: 'Close pane', category: 'terminal' },
  { key: 'F2', description: 'Rename focused tab', category: 'terminal' },
  { key: 'Shift+Alt+←/→', description: 'Reorder tab', category: 'terminal' },

  // Editor (Viewer)
  { key: 'Ctrl+E', description: 'Toggle edit mode', category: 'editor' },
  { key: 'Ctrl+S', description: 'Save file', category: 'editor' },

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
