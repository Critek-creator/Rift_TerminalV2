type InteractionType = 'click' | 'expand' | 'dismiss' | 'ignore';

interface TimestampedEntry {
  ts: number;
}

interface KindInteractions {
  clicks: TimestampedEntry[];
  expands: TimestampedEntry[];
  dismisses: TimestampedEntry[];
  ignores: TimestampedEntry[];
}

type InteractionStore = Record<string, KindInteractions>;

const STORAGE_KEY = 'rift-notif-priority-interactions';
const SEVEN_DAYS_MS = 7 * 24 * 60 * 60 * 1000;
const WEIGHTS = { click: 3, expand: 2, dismiss: 0, ignore: -1 } as const;
const MAX_PER_KIND = 200;

let saveTimer: ReturnType<typeof setTimeout> | null = null;
function debouncedSave(store: InteractionStore): void {
  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => { saveToStorage(store); saveTimer = null; }, 5000);
}

function loadFromStorage(): InteractionStore {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function saveToStorage(store: InteractionStore): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
  } catch { /* quota exceeded — degrade gracefully */ }
}

function pruneOld(entries: TimestampedEntry[], now: number): TimestampedEntry[] {
  const cutoff = now - SEVEN_DAYS_MS;
  return entries.filter((e) => e.ts >= cutoff);
}

function emptyKind(): KindInteractions {
  return { clicks: [], expands: [], dismisses: [], ignores: [] };
}

function computeScore(kind: KindInteractions, now: number): number {
  const clicks = pruneOld(kind.clicks, now);
  const expands = pruneOld(kind.expands, now);
  const ignores = pruneOld(kind.ignores, now);

  const raw =
    clicks.length * WEIGHTS.click +
    expands.length * WEIGHTS.expand +
    ignores.length * WEIGHTS.ignore;

  let recencyBoost = 0;
  if (clicks.length > 0) {
    const lastClick = Math.max(...clicks.map((c) => c.ts));
    const hoursSince = (now - lastClick) / (1000 * 60 * 60);
    recencyBoost = 10 / (hoursSince + 1);
  }

  return Math.max(0, raw + recencyBoost);
}

let enabled = $state(true);
let interactions = $state<InteractionStore>(loadFromStorage());
let _version = $state(0);

const scores = $derived.by(() => {
  void _version;
  const now = Date.now();
  const result = new Map<string, number>();
  for (const [kind, data] of Object.entries(interactions)) {
    result.set(kind, computeScore(data, now));
  }
  return result;
});

export const notifPriority = {
  recordInteraction(kind: string, action: InteractionType): void {
    if (!enabled) return;
    const entry: TimestampedEntry = { ts: Date.now() };
    const current = interactions[kind] ?? emptyKind();

    switch (action) {
      case 'click':
        current.clicks = [...current.clicks, entry];
        break;
      case 'expand':
        current.expands = [...current.expands, entry];
        break;
      case 'dismiss':
        current.dismisses = [...current.dismisses, entry];
        break;
      case 'ignore':
        current.ignores = [...current.ignores, entry];
        break;
    }

    if (current.clicks.length > MAX_PER_KIND) current.clicks = current.clicks.slice(-MAX_PER_KIND);
    if (current.expands.length > MAX_PER_KIND) current.expands = current.expands.slice(-MAX_PER_KIND);
    if (current.dismisses.length > MAX_PER_KIND) current.dismisses = current.dismisses.slice(-MAX_PER_KIND);
    if (current.ignores.length > MAX_PER_KIND) current.ignores = current.ignores.slice(-MAX_PER_KIND);

    interactions = { ...interactions, [kind]: current };
    _version++;
    debouncedSave(interactions);
  },

  getScore(kind: string): number {
    return scores.get(kind) ?? 0;
  },

  getScores(): Map<string, number> {
    return scores;
  },

  isEnabled(): boolean {
    return enabled;
  },

  setEnabled(v: boolean): void {
    enabled = v;
  },

  reset(): void {
    interactions = {};
    _version++;
    localStorage.removeItem(STORAGE_KEY);
  },

  sortByPriority<T extends { kind: string }>(items: T[]): T[] {
    if (!enabled) return items;
    return [...items].sort((a, b) => {
      const sa = scores.get(a.kind) ?? 0;
      const sb = scores.get(b.kind) ?? 0;
      return sb - sa;
    });
  },
};
