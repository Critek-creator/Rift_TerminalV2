import type { AlertRule, AlertAction } from './riftConfig';
import type { SparklineBuffer } from './SparklineBuffer';
import { kindToSeverity, type SeverityLevel } from './notifFilter';

const SEVERITY_RANK: Record<SeverityLevel, number> = {
  debug: 0,
  info: 1,
  warn: 2,
  error: 3,
};

export function evaluateRule(
  rule: AlertRule,
  buffer: SparklineBuffer,
  eventKind: string,
): boolean {
  if (!rule.enabled) return false;
  const eventSeverity = kindToSeverity(eventKind);
  if (SEVERITY_RANK[eventSeverity] < SEVERITY_RANK[rule.severity]) return false;
  const snap = buffer.snapshot();
  const windowBuckets = Math.min(Math.max(1, rule.window_secs), snap.length);
  let sum = 0;
  for (let i = snap.length - windowBuckets; i < snap.length; i++) {
    sum += snap[i];
  }
  return sum >= rule.threshold;
}

let audioCtx: AudioContext | null = null;

export function playAlertTone(): void {
  try {
    if (!audioCtx) audioCtx = new AudioContext();
    const osc = audioCtx.createOscillator();
    const gain = audioCtx.createGain();
    osc.type = 'sine';
    osc.frequency.value = 880;
    gain.gain.value = 0.15;
    osc.connect(gain);
    gain.connect(audioCtx.destination);
    osc.start();
    gain.gain.exponentialRampToValueAtTime(0.001, audioCtx.currentTime + 0.2);
    osc.stop(audioCtx.currentTime + 0.2);
  } catch {
    // Web Audio not available — silent fallback.
  }
}

export function triggerAction(action: AlertAction): { flash: boolean; promote: boolean } {
  const result = { flash: false, promote: false };
  switch (action) {
    case 'flash':
      result.flash = true;
      break;
    case 'promote':
      result.promote = true;
      break;
    case 'tone':
      playAlertTone();
      result.flash = true;
      break;
  }
  return result;
}

let idCounter = 0;

export function newAlertRuleId(): string {
  idCounter += 1;
  return `alert-${Date.now().toString(36)}-${idCounter.toString(36)}`;
}
