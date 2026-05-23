#!/usr/bin/env node
import { readFileSync } from 'fs';
import { join } from 'path';

const HOME = process.env.USERPROFILE || process.env.HOME;
const STATE_FILE = join(HOME, '.claude', 'maintenance-state.json');

try {
  const state = JSON.parse(readFileSync(STATE_FILE, 'utf8'));
  const now = Date.now();
  let overdue = 0;
  let dueSoon = 0;
  const overdueNames = [];

  for (const [key, task] of Object.entries(state.tasks)) {
    if (!task.last_run) {
      overdue++;
      overdueNames.push(key);
      continue;
    }
    const elapsed = now - new Date(task.last_run).getTime();
    const ratio = elapsed / (task.threshold_hours * 3600000);
    if (ratio >= 1.5) {
      overdue++;
      overdueNames.push(key);
    } else if (ratio >= 1.0) {
      dueSoon++;
    }
  }

  if (overdue === 0 && dueSoon === 0) process.exit(0);

  const parts = [];
  if (overdue > 0) parts.push(`${overdue} overdue`);
  if (dueSoon > 0) parts.push(`${dueSoon} due soon`);
  const top3 = overdueNames.slice(0, 3).join(', ');
  const detail = top3 ? ` (${top3})` : '';

  console.log(`⚠ MAINTENANCE: ${parts.join(', ')}${detail}. Run /aegis --maintain --dry-run to review.`);
} catch {
  process.exit(0);
}
