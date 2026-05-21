import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import TabBar from '../TabBar.svelte';
import type { SessionTab, NotifTab, ActiveSurface } from '../TabBar.svelte';

function makeSession(overrides: Partial<SessionTab> = {}): SessionTab {
  return { id: 1, title: 'Session 1', projectPath: null, ...overrides };
}

function makeNotif(overrides: Partial<NotifTab> = {}): NotifTab {
  return {
    id: 'errors',
    title: 'errors',
    icon: '⚡',
    enabled: true,
    detected: true,
    unreadCount: 0,
    lastActivityTs: null,
    ...overrides,
  };
}

const noop = () => {};
const defaultProps = {
  sessions: [makeSession()],
  notifs: [makeNotif()],
  active: { kind: 'session', id: 1 } as ActiveSurface,
  promotedId: null,
  tickNow: Date.now(),
  onActivateSession: noop,
  onActivateNotif: noop,
  onCloseSession: noop,
  onAddSession: noop,
  onToggleNotif: noop,
  onPromote: noop,
  onDemote: noop,
  onManageNotifs: noop,
  onReorderNotif: noop,
  onDetach: noop,
  detachedIds: new Set<string>(),
};

describe('TabBar', () => {
  it('renders session tabs', () => {
    render(TabBar, { props: defaultProps });
    expect(screen.getByText('Session 1')).toBeTruthy();
  });

  it('renders notification tabs that are detected and enabled', () => {
    render(TabBar, {
      props: {
        ...defaultProps,
        notifs: [
          makeNotif({ id: 'errors', title: 'errors', icon: '⚡' }),
          makeNotif({ id: 'hooks', title: 'hooks', icon: '⚓' }),
        ],
      },
    });
    expect(screen.getByText('errors')).toBeTruthy();
    expect(screen.getByText('hooks')).toBeTruthy();
  });

  it('hides notification tabs that are not detected', () => {
    render(TabBar, {
      props: {
        ...defaultProps,
        notifs: [
          makeNotif({ id: 'aegis', title: 'aegis', detected: false }),
        ],
      },
    });
    expect(screen.queryByText('aegis')).toBeNull();
  });

  it('marks the active session tab with aria-selected', () => {
    render(TabBar, { props: defaultProps });
    const tabs = screen.getAllByRole('tab');
    const activeTab = tabs.find((t) => t.getAttribute('aria-selected') === 'true');
    expect(activeTab).toBeTruthy();
    expect(activeTab?.textContent).toContain('Session 1');
  });

  it('shows unread badge when count > 0', () => {
    render(TabBar, {
      props: {
        ...defaultProps,
        notifs: [makeNotif({ unreadCount: 5 })],
      },
    });
    expect(screen.getByText('5')).toBeTruthy();
  });

  it('caps unread badge at 99+', () => {
    render(TabBar, {
      props: {
        ...defaultProps,
        notifs: [makeNotif({ unreadCount: 150 })],
      },
    });
    expect(screen.getByText('99+')).toBeTruthy();
  });

  it('renders add-session button', () => {
    render(TabBar, { props: defaultProps });
    expect(screen.getByLabelText('new tab')).toBeTruthy();
  });

  it('renders manage-notifs button', () => {
    render(TabBar, { props: defaultProps });
    expect(screen.getByLabelText('manage notification tabs')).toBeTruthy();
  });

  it('calls onCloseSession when close button clicked', () => {
    const onCloseSession = vi.fn();
    render(TabBar, {
      props: { ...defaultProps, onCloseSession },
    });
    const closeBtn = screen.getByLabelText('close tab');
    closeBtn.click();
    expect(onCloseSession).toHaveBeenCalledWith(1);
  });

  it('shows project name in multi-project mode', () => {
    render(TabBar, {
      props: {
        ...defaultProps,
        sessions: [makeSession({ projectPath: '/home/user/my-project' })],
        multiProject: true,
      },
    });
    expect(screen.getByText('Session 1 · my-project')).toBeTruthy();
  });
});
