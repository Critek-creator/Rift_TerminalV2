import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import TabBar from '../TabBar.svelte';
import type { SessionTab, NotifTab, ActiveSurface } from '../TabBar.svelte';
import type { NotifGroupState } from '../notifState.svelte';

function makeSession(overrides: Partial<SessionTab> = {}): SessionTab {
  return { id: 1, title: 'Session 1', projectPath: null, layout: { type: 'terminal', id: 1 }, ...overrides };
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

function makeGroup(tabs: NotifTab[], overrides: Partial<NotifGroupState> = {}): NotifGroupState {
  return {
    id: 'system',
    title: 'System',
    icon: '⚡',
    order: 0,
    accent: 'errors',
    tabs,
    aggregateBadge: tabs.reduce((sum, t) => sum + t.unreadCount, 0),
    ...overrides,
  };
}

const noop = () => {};
const defaultNotifs = [makeNotif()];
const defaultProps = {
  sessions: [makeSession()],
  groupedNotifs: [makeGroup(defaultNotifs)],
  active: { kind: 'session', id: 1 } as ActiveSurface,
  promotedId: null,
  tickNow: Date.now(),
  onActivateSession: noop,
  onActivateNotif: noop,
  onCloseSession: noop,
  onAddSession: noop,
  onReorderSession: noop,
  onRenameSession: noop,
  onToggleNotif: noop,
  onDemote: noop,
  onManageNotifs: noop,
  onDetach: noop,
  detachedIds: new Set<string>(),
};

describe('TabBar', () => {
  it('renders session tabs', () => {
    render(TabBar, { props: defaultProps });
    expect(screen.getByText('Session 1')).toBeTruthy();
  });

  it('renders notification groups with tabs inside', () => {
    const tabs = [
      makeNotif({ id: 'errors', title: 'errors', icon: '⚡' }),
      makeNotif({ id: 'hooks', title: 'hooks', icon: '⚓' }),
    ];
    render(TabBar, {
      props: {
        ...defaultProps,
        groupedNotifs: [makeGroup(tabs)],
      },
    });
    expect(screen.getByText('System')).toBeTruthy();
  });

  it('hides groups with no visible tabs', () => {
    render(TabBar, {
      props: {
        ...defaultProps,
        groupedNotifs: [],
      },
    });
    expect(screen.queryByText('System')).toBeNull();
  });

  it('marks the active session tab with aria-selected', () => {
    render(TabBar, { props: defaultProps });
    const tabs = screen.getAllByRole('tab');
    const activeTab = tabs.find((t) => t.getAttribute('aria-selected') === 'true');
    expect(activeTab).toBeTruthy();
    expect(activeTab?.textContent).toContain('Session 1');
  });

  it('shows aggregate unread badge on group', () => {
    const tabs = [makeNotif({ unreadCount: 5 })];
    render(TabBar, {
      props: {
        ...defaultProps,
        groupedNotifs: [makeGroup(tabs)],
      },
    });
    expect(screen.getByText('5')).toBeTruthy();
  });

  it('shows large aggregate badge count', () => {
    const tabs = [makeNotif({ unreadCount: 150 })];
    render(TabBar, {
      props: {
        ...defaultProps,
        groupedNotifs: [makeGroup(tabs)],
      },
    });
    expect(screen.getByText('150')).toBeTruthy();
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
      props: { ...defaultProps, onCloseSession, groupedNotifs: defaultProps.groupedNotifs },
    });
    const closeBtn = screen.getByLabelText('Close Session 1');
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
