/**
 * Bus Event Annotation Store
 *
 * Persists human annotations (notes + tags) on individual bus events.
 * Keyed by a synthetic envelope ID (ts:category:kind). Annotations are
 * stored in localStorage per session. The correlation engine can index
 * annotation text for cross-tab search.
 */

import type { Envelope } from './bus';

/** Allowed annotation tags — displayed as chip selectors in the UI. */
export type AnnotationTag =
  | 'root-cause'
  | 'regression'
  | 'expected'
  | 'noise'
  | 'investigate';

export const ANNOTATION_TAGS: readonly AnnotationTag[] = [
  'root-cause',
  'regression',
  'expected',
  'noise',
  'investigate',
] as const;

/** Tag display metadata for rendering chips. */
export const TAG_META: Record<AnnotationTag, { label: string; cssVar: string }> = {
  'root-cause':  { label: 'ROOT CAUSE',  cssVar: 'var(--term-red)' },
  'regression':  { label: 'REGRESSION',  cssVar: 'var(--term-red)' },
  'expected':    { label: 'EXPECTED',    cssVar: 'var(--term-green)' },
  'noise':       { label: 'NOISE',       cssVar: 'var(--amber-faint)' },
  'investigate': { label: 'INVESTIGATE', cssVar: 'var(--term-cyan)' },
};

export interface Annotation {
  /** Synthetic envelope key: ts:category:kind */
  envelopeId: string;
  /** Human-written note text. */
  note: string;
  /** Zero or more classification tags. */
  tags: AnnotationTag[];
  /** Timestamp when annotation was created/last edited. */
  createdAt: number;
  /** Copy of the annotated envelope for display in bookmarks panel. */
  envelope: Envelope;
}

const STORAGE_KEY_PREFIX = 'rift-annotations';

/** Build a deterministic key from an envelope. */
export function envelopeKey(env: Envelope): string {
  return `${env.ts}:${env.category}:${env.kind}`;
}

/**
 * Annotation store — singleton per session.
 * All mutations auto-persist to localStorage.
 */
class AnnotationStore {
  private annotations = new Map<string, Annotation>();
  private storageKey: string;
  private listeners: Array<() => void> = [];

  constructor() {
    this.storageKey = `${STORAGE_KEY_PREFIX}-${this.sessionSlug()}`;
    this.load();
  }

  private sessionSlug(): string {
    // Use today's date as a rough session grouping.
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  }

  private load(): void {
    try {
      const raw = localStorage.getItem(this.storageKey);
      if (!raw) return;
      const parsed = JSON.parse(raw) as Array<[string, Annotation]>;
      this.annotations = new Map(parsed);
    } catch {
      // Corrupted storage — start fresh.
      this.annotations = new Map();
    }
  }

  private persist(): void {
    try {
      const entries = Array.from(this.annotations.entries());
      localStorage.setItem(this.storageKey, JSON.stringify(entries));
    } catch {
      // localStorage full or unavailable — silent.
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

  /** Add or update an annotation on an envelope. */
  annotate(env: Envelope, note: string, tags: AnnotationTag[]): void {
    const id = envelopeKey(env);
    const existing = this.annotations.get(id);
    this.annotations.set(id, {
      envelopeId: id,
      note,
      tags,
      createdAt: existing?.createdAt ?? Date.now(),
      envelope: env,
    });
    this.persist();
  }

  /** Remove an annotation. */
  remove(envelopeId: string): void {
    if (this.annotations.delete(envelopeId)) {
      this.persist();
    }
  }

  /** Get annotation for a specific envelope, or null. */
  get(envelopeId: string): Annotation | null {
    return this.annotations.get(envelopeId) ?? null;
  }

  /** Get annotation by envelope object. */
  getForEnvelope(env: Envelope): Annotation | null {
    return this.get(envelopeKey(env));
  }

  /** Check if an envelope has an annotation. */
  has(envelopeId: string): boolean {
    return this.annotations.has(envelopeId);
  }

  /** All annotations, newest first. */
  getAll(): Annotation[] {
    return Array.from(this.annotations.values()).sort(
      (a, b) => b.createdAt - a.createdAt
    );
  }

  /** Count of annotations. */
  get count(): number {
    return this.annotations.size;
  }

  /** Search annotations by note text (case-insensitive substring). */
  search(query: string): Annotation[] {
    const q = query.toLowerCase();
    return this.getAll().filter(
      (a) =>
        a.note.toLowerCase().includes(q) ||
        a.tags.some((t) => t.includes(q)) ||
        a.envelope.kind.toLowerCase().includes(q)
    );
  }
}

/** Singleton annotation store instance. */
export const annotationStore = new AnnotationStore();
