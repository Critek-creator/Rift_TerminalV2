/**
 * Bus Event Bookmarks & Saved Filter Queries
 *
 * Bookmarks: individual bus events saved for later reference.
 * Persisted to localStorage keyed by date.
 *
 * Saved Queries: named filter predicates (e.g. "hook failures",
 * "fs changes in src/"). Persisted to localStorage (frontend-only;
 * rift config integration deferred until Rust-side schema catches up).
 */

import type { Category, Envelope } from './bus';
import type { SeverityLevel } from './notifFilter';

// ---------------------------------------------------------------------------
// Bookmarks
// ---------------------------------------------------------------------------

export interface Bookmark {
  /** Same synthetic key as annotations: ts:category:kind */
  envelopeId: string;
  /** Copy of the bookmarked envelope. */
  envelope: Envelope;
  /** When the bookmark was created. */
  createdAt: number;
}

const BOOKMARKS_KEY_PREFIX = 'rift-bookmarks';

function bookmarksStorageKey(): string {
  const d = new Date();
  return `${BOOKMARKS_KEY_PREFIX}-${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** Build the same deterministic key used by annotations. */
export function envelopeKey(env: Envelope): string {
  return `${env.ts}:${env.category}:${env.kind}`;
}

class BookmarkStore {
  private bookmarks = new Map<string, Bookmark>();
  private storageKey: string;
  private listeners: Array<() => void> = [];

  constructor() {
    this.storageKey = bookmarksStorageKey();
    this.load();
  }

  private load(): void {
    try {
      const raw = localStorage.getItem(this.storageKey);
      if (!raw) return;
      const parsed = JSON.parse(raw) as Array<[string, Bookmark]>;
      this.bookmarks = new Map(parsed);
    } catch {
      this.bookmarks = new Map();
    }
  }

  private persist(): void {
    try {
      const entries = Array.from(this.bookmarks.entries());
      localStorage.setItem(this.storageKey, JSON.stringify(entries));
    } catch {
      // localStorage full — silent.
    }
    for (const fn of this.listeners) fn();
  }

  /** Subscribe to store changes. Returns unsubscribe function. */
  onChange(fn: () => void): () => void {
    this.listeners.push(fn);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== fn);
    };
  }

  /** Bookmark an envelope. */
  bookmark(env: Envelope): void {
    const id = envelopeKey(env);
    if (this.bookmarks.has(id)) return;
    this.bookmarks.set(id, {
      envelopeId: id,
      envelope: env,
      createdAt: Date.now(),
    });
    this.persist();
  }

  /** Remove a bookmark. */
  unbookmark(envelopeId: string): void {
    if (this.bookmarks.delete(envelopeId)) {
      this.persist();
    }
  }

  /** Toggle bookmark state. Returns true if now bookmarked. */
  toggle(env: Envelope): boolean {
    const id = envelopeKey(env);
    if (this.bookmarks.has(id)) {
      this.bookmarks.delete(id);
      this.persist();
      return false;
    }
    this.bookmarks.set(id, {
      envelopeId: id,
      envelope: env,
      createdAt: Date.now(),
    });
    this.persist();
    return true;
  }

  /** Check if an envelope is bookmarked. */
  isBookmarked(envelopeId: string): boolean {
    return this.bookmarks.has(envelopeId);
  }

  /** Check by envelope object. */
  isEnvelopeBookmarked(env: Envelope): boolean {
    return this.isBookmarked(envelopeKey(env));
  }

  /** All bookmarks, newest first. */
  getAll(): Bookmark[] {
    return Array.from(this.bookmarks.values()).sort(
      (a, b) => b.createdAt - a.createdAt
    );
  }

  /** Count of bookmarks. */
  get count(): number {
    return this.bookmarks.size;
  }
}

/** Singleton bookmark store. */
export const bookmarkStore = new BookmarkStore();

// ---------------------------------------------------------------------------
// Saved Filter Queries
// ---------------------------------------------------------------------------

/** A named filter predicate that users can save and recall. */
export interface SavedQuery {
  /** Unique identifier. */
  id: string;
  /** Human-readable name (e.g. "hook failures"). */
  name: string;
  /** Category filter — undefined means all categories. */
  categories?: Category[];
  /** Kind substring match (case-insensitive). */
  kindPattern?: string;
  /** Payload substring match (case-insensitive). */
  payloadPattern?: string;
  /** Minimum severity level. */
  minSeverity?: SeverityLevel;
  /** When the query was created. */
  createdAt: number;
}

const QUERIES_STORAGE_KEY = 'rift-saved-queries';

/** Default built-in queries shipped with Rift. */
const DEFAULT_QUERIES: SavedQuery[] = [
  {
    id: 'builtin-hook-failures',
    name: 'Hook failures',
    categories: ['hook'],
    kindPattern: 'error',
    createdAt: 0,
  },
  {
    id: 'builtin-fs-src',
    name: 'FS changes in src/',
    categories: ['fs'],
    payloadPattern: 'src/',
    createdAt: 0,
  },
  {
    id: 'builtin-errors-all',
    name: 'All errors',
    minSeverity: 'error',
    createdAt: 0,
  },
];

class SavedQueryStore {
  private queries = new Map<string, SavedQuery>();
  private listeners: Array<() => void> = [];

  constructor() {
    this.load();
  }

  private load(): void {
    try {
      const raw = localStorage.getItem(QUERIES_STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw) as Array<[string, SavedQuery]>;
        this.queries = new Map(parsed);
      }
    } catch {
      this.queries = new Map();
    }
    // Ensure defaults are present (keyed by id so user edits survive).
    for (const dq of DEFAULT_QUERIES) {
      if (!this.queries.has(dq.id)) {
        this.queries.set(dq.id, dq);
      }
    }
  }

  private persist(): void {
    try {
      const entries = Array.from(this.queries.entries());
      localStorage.setItem(QUERIES_STORAGE_KEY, JSON.stringify(entries));
    } catch {
      // silent
    }
    for (const fn of this.listeners) fn();
  }

  /** Subscribe to store changes. Returns unsubscribe function. */
  onChange(fn: () => void): () => void {
    this.listeners.push(fn);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== fn);
    };
  }

  /** Save a new query. */
  save(query: Omit<SavedQuery, 'id' | 'createdAt'>): SavedQuery {
    const id = `sq-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
    const full: SavedQuery = { ...query, id, createdAt: Date.now() };
    this.queries.set(id, full);
    this.persist();
    return full;
  }

  /** Update an existing query. */
  update(id: string, patch: Partial<Omit<SavedQuery, 'id' | 'createdAt'>>): void {
    const existing = this.queries.get(id);
    if (!existing) return;
    this.queries.set(id, { ...existing, ...patch });
    this.persist();
  }

  /** Remove a query. */
  remove(id: string): void {
    if (this.queries.delete(id)) {
      this.persist();
    }
  }

  /** Get a query by id. */
  get(id: string): SavedQuery | undefined {
    return this.queries.get(id);
  }

  /** All queries, newest first (builtins at the end). */
  getAll(): SavedQuery[] {
    return Array.from(this.queries.values()).sort((a, b) => {
      // Builtins (createdAt === 0) sort last.
      if (a.createdAt === 0 && b.createdAt !== 0) return 1;
      if (b.createdAt === 0 && a.createdAt !== 0) return -1;
      return b.createdAt - a.createdAt;
    });
  }

  /** Count of saved queries. */
  get count(): number {
    return this.queries.size;
  }

  /** Test whether an envelope matches a saved query. */
  matches(env: Envelope, query: SavedQuery): boolean {
    if (query.categories && query.categories.length > 0) {
      if (!query.categories.includes(env.category)) return false;
    }
    if (query.kindPattern) {
      if (!env.kind.toLowerCase().includes(query.kindPattern.toLowerCase())) return false;
    }
    if (query.payloadPattern) {
      const payloadStr = typeof env.payload === 'string'
        ? env.payload
        : JSON.stringify(env.payload ?? '');
      if (!payloadStr.toLowerCase().includes(query.payloadPattern.toLowerCase())) return false;
    }
    if (query.minSeverity) {
      const RANK: Record<string, number> = { debug: 0, info: 1, warn: 2, error: 3 };
      const envSev = env.kind.toLowerCase().includes('error') || env.kind.toLowerCase().includes('failed')
        ? 3
        : env.kind.toLowerCase().includes('warn')
          ? 2
          : env.kind.toLowerCase().includes('debug')
            ? 0
            : 1;
      if (envSev < (RANK[query.minSeverity] ?? 1)) return false;
    }
    return true;
  }
}

/** Singleton saved query store. */
export const savedQueryStore = new SavedQueryStore();
