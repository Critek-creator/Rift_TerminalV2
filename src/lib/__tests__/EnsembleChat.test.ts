import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';

// Mock Tauri core before importing the component
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  Channel: vi.fn(() => ({ onmessage: null })),
}));

import EnsembleChat from '../EnsembleChat.svelte';

describe('EnsembleChat', () => {
  it('renders with the ENSEMBLE label in the header', () => {
    render(EnsembleChat, { props: { popoutId: 1 } });

    expect(screen.getByText('ENSEMBLE')).toBeTruthy();
  });

  it('shows two pane headers with model selector buttons', () => {
    const { container } = render(EnsembleChat, { props: { popoutId: 1 } });

    const paneHeaders = container.querySelectorAll('.pane-header');
    expect(paneHeaders).toHaveLength(2);
  });

  it('shows the critique toggle checkbox', () => {
    const { container } = render(EnsembleChat, { props: { popoutId: 1 } });

    const checkbox = container.querySelector('.critique-toggle input[type="checkbox"]');
    expect(checkbox).toBeTruthy();

    expect(screen.getByText('Critique')).toBeTruthy();
  });

  it('shows the send button', () => {
    render(EnsembleChat, { props: { popoutId: 1 } });

    const sendBtn = screen.getByText('Send');
    expect(sendBtn).toBeTruthy();
    expect(sendBtn.tagName).toBe('BUTTON');
  });

  it('shows empty-state instruction text when no messages and no models selected', () => {
    render(EnsembleChat, { props: { popoutId: 1 } });

    // Default empty state shows "Select a model" in both panes
    const empties = screen.getAllByText('Select a model');
    expect(empties.length).toBe(2);
  });

  it('shows model A and model B default badges', () => {
    render(EnsembleChat, { props: { popoutId: 1 } });

    expect(screen.getByText('Model A')).toBeTruthy();
    expect(screen.getByText('Model B')).toBeTruthy();
  });

  it('renders the versus separator in the header', () => {
    render(EnsembleChat, { props: { popoutId: 1 } });

    expect(screen.getByText('vs')).toBeTruthy();
  });

  it('has a textarea for input', () => {
    const { container } = render(EnsembleChat, { props: { popoutId: 1 } });

    const textarea = container.querySelector('.input-area textarea');
    expect(textarea).toBeTruthy();
  });
});
