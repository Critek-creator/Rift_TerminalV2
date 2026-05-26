#!/usr/bin/env node
// Kill any process holding port 1420 (Vite dev server) so a fresh `vite`
// can bind without EADDRINUSE. Runs as `predev` npm hook — silent on
// success, tolerates missing commands / no listeners.

import { execSync } from 'node:child_process';

const PORT = 1420;

try {
  if (process.platform === 'win32') {
    const out = execSync(
      `powershell -NoProfile -Command "Get-NetTCPConnection -LocalPort ${PORT} -ErrorAction Stop | Select-Object -ExpandProperty OwningProcess"`,
      { encoding: 'utf8', stdio: ['pipe', 'pipe', 'ignore'] },
    );
    const pids = [...new Set(out.trim().split(/\r?\n/).filter(Boolean))];
    for (const pid of pids) {
      try {
        execSync(`taskkill /PID ${pid} /F`, { stdio: 'ignore' });
        console.log(`[kill-port] killed PID ${pid} on port ${PORT}`);
      } catch { /* already gone */ }
    }
  } else {
    execSync(`lsof -ti:${PORT} | xargs -r kill -9`, { stdio: 'ignore' });
  }
} catch {
  // No listeners on the port — nothing to kill.
}
