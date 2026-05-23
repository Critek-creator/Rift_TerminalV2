#!/usr/bin/env node
import { readFileSync, writeFileSync } from 'fs';
import { join } from 'path';

const HOME = process.env.USERPROFILE || process.env.HOME;
const STATE_FILE = join(HOME, '.claude', 'maintenance-state.json');

const command = process.argv[2];
const taskKey = process.argv[3];

function usage() {
  console.error('Usage:');
  console.error('  maintenance-update.mjs touch <task_key>     — set last_run to now');
  console.error('  maintenance-update.mjs reset <task_key>     — clear last_run');
  console.error('  maintenance-update.mjs bootstrap            — set all last_run to now');
  console.error('  maintenance-update.mjs bootstrap-except <k> — bootstrap all except <k> (comma-sep)');
  process.exit(1);
}

if (!command) usage();

try {
  const state = JSON.parse(readFileSync(STATE_FILE, 'utf8'));
  const now = new Date().toISOString();

  if (command === 'touch') {
    if (!taskKey || !state.tasks[taskKey]) {
      console.error(`Unknown task: ${taskKey}`);
      console.error('Available:', Object.keys(state.tasks).join(', '));
      process.exit(1);
    }
    state.tasks[taskKey].last_run = now;
    console.log(`UPDATED: ${taskKey} last_run=${now}`);

  } else if (command === 'reset') {
    if (!taskKey || !state.tasks[taskKey]) {
      console.error(`Unknown task: ${taskKey}`);
      process.exit(1);
    }
    state.tasks[taskKey].last_run = null;
    console.log(`RESET: ${taskKey} last_run=null`);

  } else if (command === 'bootstrap') {
    let count = 0;
    for (const key of Object.keys(state.tasks)) {
      state.tasks[key].last_run = now;
      count++;
    }
    state.created = now;
    console.log(`BOOTSTRAPPED: ${count} tasks set to ${now}`);

  } else if (command === 'bootstrap-except') {
    const except = (taskKey || '').split(',').map(k => k.trim());
    let bootstrapped = 0;
    let skipped = 0;
    for (const key of Object.keys(state.tasks)) {
      if (except.includes(key)) {
        skipped++;
      } else {
        state.tasks[key].last_run = now;
        bootstrapped++;
      }
    }
    state.created = now;
    console.log(`BOOTSTRAPPED: ${bootstrapped} tasks set to ${now}, ${skipped} skipped (${except.join(', ')})`);

  } else {
    usage();
  }

  writeFileSync(STATE_FILE, JSON.stringify(state, null, 2) + '\n');
} catch (e) {
  if (e.code === 'ENOENT') {
    console.error('State file not found. Run /aegis --maintain to initialize.');
    process.exit(2);
  }
  console.error(e.message);
  process.exit(1);
}
