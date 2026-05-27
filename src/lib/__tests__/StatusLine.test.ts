import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import StatusLine from '../StatusLine.svelte';

describe('StatusLine', () => {
  it('renders all segment labels with default dash values', () => {
    render(StatusLine);

    expect(screen.getByText('DIR')).toBeTruthy();
    expect(screen.getByText('MODEL')).toBeTruthy();
    expect(screen.getByText('CTX')).toBeTruthy();
    expect(screen.getByText('SESSION')).toBeTruthy();
    expect(screen.getByText('SKILL')).toBeTruthy();
    expect(screen.getByText('GIT')).toBeTruthy();
    expect(screen.getByText('REPO')).toBeTruthy();
    expect(screen.getByText('USE')).toBeTruthy();
    expect(screen.getByText('WEEK')).toBeTruthy();
    expect(screen.getByText('EFFORT')).toBeTruthy();
  });

  it('displays provided values in the correct segments', () => {
    render(StatusLine, {
      props: {
        dir: '~/projects/rift',
        git: 'main',
        repo: 'Critek/Rift',
        model: 'opus-4',
        skill: 'aegis',
      },
    });

    expect(screen.getByText('~/projects/rift')).toBeTruthy();
    expect(screen.getByText('main')).toBeTruthy();
    expect(screen.getByText('Critek/Rift')).toBeTruthy();
    expect(screen.getByText('opus-4')).toBeTruthy();
    expect(screen.getByText('aegis')).toBeTruthy();
  });

  it('has role="status" and aria-live for accessibility', () => {
    const { container } = render(StatusLine);
    const footer = container.querySelector('footer.statusline');
    expect(footer?.getAttribute('role')).toBe('status');
    expect(footer?.getAttribute('aria-live')).toBe('polite');
  });

  it('hides segments when visibility config disables them', () => {
    render(StatusLine, {
      props: {
        dir: 'visible-dir',
        model: 'visible-model',
        visibility: {
          show_dir: false,
          show_model: true,
          show_ctx: false,
          show_session: false,
          show_skill: false,
          show_thinking: false,
          show_effort: false,
          show_git: false,
          show_repo: false,
          show_session_use: false,
          show_week: false,
          color_overrides: {},
        },
      },
    });

    expect(screen.queryByText('visible-dir')).toBeNull();
    expect(screen.getByText('visible-model')).toBeTruthy();
    expect(screen.queryByText('DIR')).toBeNull();
    expect(screen.getByText('MODEL')).toBeTruthy();
  });

  it('renders two rows', () => {
    const { container } = render(StatusLine);
    const rows = container.querySelectorAll('.row');
    expect(rows).toHaveLength(2);
  });
});
