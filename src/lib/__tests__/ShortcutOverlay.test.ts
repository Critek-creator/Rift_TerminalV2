import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import ShortcutOverlay from '../ShortcutOverlay.svelte';

describe('ShortcutOverlay', () => {
  it('renders as a dialog with correct ARIA attributes', () => {
    render(ShortcutOverlay, { props: { onclose: vi.fn() } });

    const dialog = screen.getByRole('dialog');
    expect(dialog).toBeTruthy();
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.getAttribute('aria-label')).toBe('Keyboard shortcuts');
  });

  it('displays the header title', () => {
    render(ShortcutOverlay, { props: { onclose: vi.fn() } });
    expect(screen.getByText('KEYBOARD SHORTCUTS')).toBeTruthy();
  });

  it('renders keybinding entries with key and description', () => {
    render(ShortcutOverlay, { props: { onclose: vi.fn() } });
    expect(screen.getByText('Ctrl+K')).toBeTruthy();
    expect(screen.getByText('Command palette')).toBeTruthy();
    expect(screen.getByText('Ctrl+B')).toBeTruthy();
    expect(screen.getByText('Toggle cockpit panel')).toBeTruthy();
  });

  it('renders category labels', () => {
    render(ShortcutOverlay, { props: { onclose: vi.fn() } });
    expect(screen.getByText('NAVIGATION')).toBeTruthy();
    expect(screen.getByText('TERMINAL')).toBeTruthy();
  });

  it('has an ESC close button', () => {
    render(ShortcutOverlay, { props: { onclose: vi.fn() } });
    expect(screen.getByText('ESC')).toBeTruthy();
  });

  it('calls onclose when ESC button is clicked', async () => {
    const onclose = vi.fn();
    render(ShortcutOverlay, { props: { onclose } });

    const escBtn = screen.getByText('ESC');
    escBtn.click();
    expect(onclose).toHaveBeenCalledOnce();
  });

  it('calls onclose when backdrop is clicked', async () => {
    const onclose = vi.fn();
    const { container } = render(ShortcutOverlay, { props: { onclose } });

    const backdrop = container.querySelector('.overlay-backdrop');
    backdrop?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(onclose).toHaveBeenCalledOnce();
  });
});
