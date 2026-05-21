/**
 * McpWaterfallStore — tracks MCP tool call spans for waterfall timeline rendering.
 *
 * Subscribes to bus events: on mcp.invoke -> open span, on mcp.response -> close span.
 * Maintains a circular buffer of the last 200 spans for the waterfall view.
 */

import type { Envelope } from './bus';

/** Tool tier classification for color coding. */
export type ToolTier = 'read' | 'mutate' | 'inspect';

/** Span status lifecycle. */
export type SpanStatus = 'pending' | 'success' | 'error';

/** A single MCP tool call span with timing data. */
export interface McpCallSpan {
  /** Unique span identifier (matches request_id from bus envelope). */
  readonly id: string;
  /** Tool name (e.g. bus_history, pty_input). */
  readonly tool: string;
  /** Classified tier for color coding. */
  readonly tier: ToolTier;
  /** Epoch ms when the request was sent. */
  readonly requestTime: number;
  /** Epoch ms when the response arrived, or null if still pending. */
  responseTime: number | null;
  /** Computed duration in ms, or null if still pending. */
  durationMs: number | null;
  /** Correlation group from envelope, if present. */
  readonly correlationGroup: string | null;
  /** Current status of the span. */
  status: SpanStatus;
  /** Raw request payload for detail view. */
  readonly requestPayload: unknown;
  /** Raw response payload for detail view, populated on close. */
  responsePayload: unknown;
}

const BUFFER_SIZE = 200;

/** Timeout threshold in ms — bars approaching this get a warning gradient. */
export const TIMEOUT_WARN_MS = 4000;
export const TIMEOUT_MS = 5000;

/** Classification maps for tool tiers. */
const READ_TOOLS = new Set([
  'bus_history', 'bus_tail', 'git_status', 'aegis_state',
  'fs_read', 'fs_tree', 'todo_scan', 'pty_list',
  'cockpit_state', 'notif_tabs', 'rift_status', 'rift_diagnose',
]);

const MUTATE_TOOLS = new Set([
  'pty_input', 'fs_write', 'git_action', 'bus_publish',
  'rift_config_set',
]);

const INSPECT_TOOLS = new Set([
  'dom_snapshot', 'screenshot', 'js_eval', 'pty_read',
  'simulate_click', 'simulate_drag',
]);

/** Classify a tool name into a tier. Falls back to 'read' for unknown tools. */
export function classifyTier(tool: string): ToolTier {
  if (MUTATE_TOOLS.has(tool)) return 'mutate';
  if (INSPECT_TOOLS.has(tool)) return 'inspect';
  if (READ_TOOLS.has(tool)) return 'read';
  // Heuristic fallback: tools containing write/create/delete/action -> mutate
  const lower = tool.toLowerCase();
  if (lower.includes('write') || lower.includes('create') || lower.includes('delete') || lower.includes('action')) {
    return 'mutate';
  }
  if (lower.includes('snapshot') || lower.includes('eval') || lower.includes('simulate')) {
    return 'inspect';
  }
  return 'read';
}

/** CSS variable name for a given tier. */
export function tierCssVar(tier: ToolTier): string {
  switch (tier) {
    case 'read': return '--term-blue';
    case 'mutate': return '--term-red';
    case 'inspect': return '--term-cyan';
  }
}

export class McpWaterfallStore {
  private spans: McpCallSpan[] = [];
  private pendingById = new Map<string, McpCallSpan>();
  private nextSyntheticId = 0;

  /** Process an incoming bus envelope. Returns true if the span list changed. */
  processEnvelope(env: Envelope): boolean {
    const kind = env.kind.toLowerCase();
    const p = env.payload as Record<string, unknown> | null;
    const tool = this.extractTool(p);
    const reqId = String(p?.request_id ?? p?.id ?? '');

    if (kind.includes('invoke') || kind.includes('call') || kind.includes('request')) {
      return this.openSpan(reqId, tool, env);
    }
    if (kind.includes('response') || kind.includes('result') || kind.includes('error') || kind.includes('fail')) {
      return this.closeSpan(reqId, env, kind.includes('error') || kind.includes('fail'));
    }
    return false;
  }

  private openSpan(reqId: string, tool: string, env: Envelope): boolean {
    const id = reqId || `syn_${this.nextSyntheticId++}`;
    const span: McpCallSpan = {
      id,
      tool: tool || 'unknown',
      tier: classifyTier(tool),
      requestTime: env.ts,
      responseTime: null,
      durationMs: null,
      correlationGroup: env.correlation_id ?? null,
      status: 'pending',
      requestPayload: env.payload,
      responsePayload: null,
    };
    this.pendingById.set(id, span);
    this.pushSpan(span);
    return true;
  }

  private closeSpan(reqId: string, env: Envelope, isError: boolean): boolean {
    if (!reqId) return false;
    const span = this.pendingById.get(reqId);
    if (!span) return false;
    this.pendingById.delete(reqId);
    span.responseTime = env.ts;
    span.durationMs = env.ts - span.requestTime;
    span.status = isError ? 'error' : 'success';
    span.responsePayload = env.payload;
    return true;
  }

  private pushSpan(span: McpCallSpan): void {
    this.spans.push(span);
    if (this.spans.length > BUFFER_SIZE * 2) {
      this.spans = this.spans.slice(-BUFFER_SIZE);
    }
  }

  /** Evict stale pending spans older than 30s. */
  tick(): void {
    const cutoff = Date.now() - 30_000;
    for (const [id, span] of this.pendingById.entries()) {
      if (span.requestTime < cutoff) {
        span.status = 'error';
        span.responseTime = span.requestTime + 30_000;
        span.durationMs = 30_000;
        this.pendingById.delete(id);
      }
    }
  }

  /** Get a snapshot of all spans (up to BUFFER_SIZE, newest last). */
  snapshot(): McpCallSpan[] {
    return this.spans.slice(-BUFFER_SIZE);
  }

  /** Get current pending count. */
  get pendingCount(): number {
    return this.pendingById.size;
  }

  /** Get total span count. */
  get totalCount(): number {
    return this.spans.length;
  }

  /** Reset all state. */
  reset(): void {
    this.spans = [];
    this.pendingById.clear();
    this.nextSyntheticId = 0;
  }

  private extractTool(payload: unknown): string {
    if (!payload || typeof payload !== 'object') return '';
    const p = payload as Record<string, unknown>;
    return String(p.tool ?? p.tool_name ?? p.name ?? '');
  }
}
