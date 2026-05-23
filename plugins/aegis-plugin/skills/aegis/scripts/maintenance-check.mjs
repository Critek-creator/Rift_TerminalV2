#!/usr/bin/env node
import { readFileSync } from 'fs';
import { join } from 'path';

const HOME = process.env.USERPROFILE || process.env.HOME;
const STATE_FILE = join(HOME, '.claude', 'maintenance-state.json');

try {
  const state = JSON.parse(readFileSync(STATE_FILE, 'utf8'));
  const now = Date.now();
  const results = { overdue: [], due_soon: [], fresh: [], never_run: [] };

  for (const [key, task] of Object.entries(state.tasks)) {
    const thresholdMs = task.threshold_hours * 3600000;
    const entry = { key, label: task.label, category: task.category, skill: task.skill, description: task.description, threshold_hours: task.threshold_hours };

    if (!task.last_run) {
      results.never_run.push({ ...entry, staleness_hours: null, ratio: null });
      continue;
    }

    const lastRunMs = new Date(task.last_run).getTime();
    const elapsed = now - lastRunMs;
    const stalenessHours = Math.round(elapsed / 3600000);
    const ratio = Math.round((elapsed / thresholdMs) * 100) / 100;

    const classified = { ...entry, staleness_hours: stalenessHours, last_run: task.last_run, ratio };

    if (ratio >= 1.5) {
      results.overdue.push(classified);
    } else if (ratio >= 1.0) {
      results.due_soon.push(classified);
    } else {
      results.fresh.push(classified);
    }
  }

  results.overdue.sort((a, b) => b.ratio - a.ratio);
  results.due_soon.sort((a, b) => b.ratio - a.ratio);

  const summary = {
    total: Object.keys(state.tasks).length,
    overdue: results.overdue.length,
    due_soon: results.due_soon.length,
    fresh: results.fresh.length,
    never_run: results.never_run.length,
    bootstrapped: !!state.created
  };

  console.log(JSON.stringify({ summary, ...results }, null, 2));
} catch (e) {
  if (e.code === 'ENOENT') {
    console.log('STATE_FILE_MISSING');
    process.exit(2);
  }
  console.error(e.message);
  process.exit(1);
}
