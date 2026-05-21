import { vi, afterEach } from 'vitest';
import { cleanup } from '@testing-library/svelte';

afterEach(() => { cleanup(); });

// Global Tauri API mocks for component tests.
// NOTE: @tauri-apps/api/core is NOT mocked here — bus.test.ts provides
// its own Channel mock with introspection. Mocking it globally conflicts.

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
  emit: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@tauri-apps/api/app', () => ({
  getName: vi.fn().mockResolvedValue('Rift'),
  getVersion: vi.fn().mockResolvedValue('0.1.10'),
}));

vi.mock('@tauri-apps/api/window', () => {
  const win = {
    label: 'main',
    setDecorations: vi.fn(),
    setTitle: vi.fn(),
    center: vi.fn(),
    show: vi.fn(),
    hide: vi.fn(),
    close: vi.fn(),
    setSize: vi.fn(),
    setPosition: vi.fn(),
    outerPosition: vi.fn().mockResolvedValue({ x: 100, y: 100 }),
    outerSize: vi.fn().mockResolvedValue({ width: 1100, height: 700 }),
    onCloseRequested: vi.fn().mockResolvedValue(() => {}),
    onMoved: vi.fn().mockResolvedValue(() => {}),
    onResized: vi.fn().mockResolvedValue(() => {}),
  };
  return {
    getCurrentWindow: vi.fn(() => win),
    Window: vi.fn(() => win),
    currentMonitor: vi.fn().mockResolvedValue({ size: { width: 1920, height: 1080 }, position: { x: 0, y: 0 } }),
  };
});

vi.mock('@tauri-apps/plugin-updater', () => ({
  check: vi.fn().mockResolvedValue(null),
}));
