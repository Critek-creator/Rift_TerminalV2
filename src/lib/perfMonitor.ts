/**
 * perfMonitor.ts — lightweight frontend performance measurement harness.
 *
 * Tracks frame timing (fps, long frames), bus event throughput, and
 * long tasks. Exposes metrics via window.__RIFT_PERF__ for baseline
 * capture during /aegis --perf sessions.
 *
 * Zero cost when not started. Call start() to begin, stop() to end,
 * report() for a snapshot. Designed to be removable after perf work.
 */

interface FrameStats {
  totalFrames: number;
  longFrames16: number;  // >16.67ms (missed 60fps budget)
  longFrames50: number;  // >50ms (long task territory)
  maxFrameMs: number;
  avgFrameMs: number;
  fps: number;           // rolling 1-second fps
}

interface BusStats {
  totalEvents: number;
  eventsPerSec: number;  // rolling 1-second rate
  byCategory: Record<string, number>;
}

interface LongTaskStats {
  count: number;
  totalDurationMs: number;
  maxDurationMs: number;
}

export interface PerfReport {
  durationMs: number;
  frames: FrameStats;
  bus: BusStats;
  longTasks: LongTaskStats;
  timestamp: string;
}

let _running = false;
let _startTs = 0;
let _rafId: number | null = null;
let _lastFrameTs = 0;
let _frameDurations: number[] = [];
let _recentFrameTimes: number[] = [];

// Bus event tracking
let _busTotal = 0;
let _busByCategory: Record<string, number> = {};
let _busRecentTimes: number[] = [];

// Long task tracking
let _longTaskCount = 0;
let _longTaskTotalMs = 0;
let _longTaskMaxMs = 0;
let _longTaskObserver: PerformanceObserver | null = null;

function frameLoop(ts: number): void {
  if (!_running) return;
  if (_lastFrameTs > 0) {
    const delta = ts - _lastFrameTs;
    _frameDurations.push(delta);
    _recentFrameTimes.push(ts);
    // Keep only last 2 seconds of frame times for rolling fps
    const cutoff = ts - 2000;
    while (_recentFrameTimes.length > 0 && _recentFrameTimes[0] < cutoff) {
      _recentFrameTimes.shift();
    }
  }
  _lastFrameTs = ts;
  _rafId = requestAnimationFrame(frameLoop);
}

function buildFrameStats(): FrameStats {
  const frames = _frameDurations;
  const total = frames.length;
  if (total === 0) {
    return { totalFrames: 0, longFrames16: 0, longFrames50: 0, maxFrameMs: 0, avgFrameMs: 0, fps: 0 };
  }
  let sum = 0;
  let max = 0;
  let long16 = 0;
  let long50 = 0;
  for (const d of frames) {
    sum += d;
    if (d > max) max = d;
    if (d > 16.67) long16++;
    if (d > 50) long50++;
  }
  const now = performance.now();
  const recentCount = _recentFrameTimes.filter(t => t > now - 1000).length;
  return {
    totalFrames: total,
    longFrames16: long16,
    longFrames50: long50,
    maxFrameMs: Math.round(max * 100) / 100,
    avgFrameMs: Math.round((sum / total) * 100) / 100,
    fps: recentCount,
  };
}

function buildBusStats(): BusStats {
  const now = Date.now();
  const cutoff = now - 1000;
  while (_busRecentTimes.length > 0 && _busRecentTimes[0] < cutoff) {
    _busRecentTimes.shift();
  }
  return {
    totalEvents: _busTotal,
    eventsPerSec: _busRecentTimes.length,
    byCategory: { ..._busByCategory },
  };
}

function buildReport(): PerfReport {
  return {
    durationMs: _running ? performance.now() - _startTs : 0,
    frames: buildFrameStats(),
    bus: buildBusStats(),
    longTasks: {
      count: _longTaskCount,
      totalDurationMs: Math.round(_longTaskTotalMs * 100) / 100,
      maxDurationMs: Math.round(_longTaskMaxMs * 100) / 100,
    },
    timestamp: new Date().toISOString(),
  };
}

export function recordBusEvent(category: string): void {
  if (!_running) return;
  _busTotal++;
  _busByCategory[category] = (_busByCategory[category] ?? 0) + 1;
  _busRecentTimes.push(Date.now());
}

export function start(): void {
  if (_running) return;
  _running = true;
  _startTs = performance.now();
  _lastFrameTs = 0;
  _frameDurations = [];
  _recentFrameTimes = [];
  _busTotal = 0;
  _busByCategory = {};
  _busRecentTimes = [];
  _longTaskCount = 0;
  _longTaskTotalMs = 0;
  _longTaskMaxMs = 0;

  _rafId = requestAnimationFrame(frameLoop);

  // Long task observer (Chrome/Edge — not available in all runtimes)
  if (typeof PerformanceObserver !== 'undefined') {
    try {
      _longTaskObserver = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          _longTaskCount++;
          _longTaskTotalMs += entry.duration;
          if (entry.duration > _longTaskMaxMs) _longTaskMaxMs = entry.duration;
        }
      });
      _longTaskObserver.observe({ type: 'longtask', buffered: true });
    } catch {
      // longtask not supported in this runtime
    }
  }

  const w = window as unknown as Record<string, unknown>;
  w.__RIFT_PERF__ = { report: buildReport, stop, start, isRunning: () => _running };
  console.log('[perfMonitor] started — call __RIFT_PERF__.report() for metrics');
}

export function stop(): void {
  if (!_running) return;
  _running = false;
  if (_rafId !== null) {
    cancelAnimationFrame(_rafId);
    _rafId = null;
  }
  if (_longTaskObserver) {
    _longTaskObserver.disconnect();
    _longTaskObserver = null;
  }
  console.log('[perfMonitor] stopped');
}

export function report(): PerfReport {
  return buildReport();
}
