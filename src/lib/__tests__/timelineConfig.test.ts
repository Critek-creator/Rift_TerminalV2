import { describe, it, expect } from 'vitest';
import { defaultTimelineConfig } from '../riftConfig';

// Unit tests for the TimelineConfig default-sources helper.
// Verifies the CORE-on / opt-in-off contract without touching any Svelte
// component internals or Tauri IPC.

describe('defaultTimelineConfig', () => {
  it('returns CORE sources ON by default', () => {
    const cfg = defaultTimelineConfig();
    expect(cfg.show_commands).toBe(true);
    expect(cfg.show_errors).toBe(true);
  });

  it('returns all opt-in sources OFF by default', () => {
    const cfg = defaultTimelineConfig();
    expect(cfg.show_agents).toBe(false);
    expect(cfg.show_hooks).toBe(false);
    expect(cfg.show_fs).toBe(false);
    expect(cfg.show_llm_cost).toBe(false);
    expect(cfg.show_mcp).toBe(false);
  });

  it('returns exactly 7 boolean fields (no extras, no missing)', () => {
    const cfg = defaultTimelineConfig();
    const keys = Object.keys(cfg);
    expect(keys).toHaveLength(7);
    for (const v of Object.values(cfg)) {
      expect(typeof v).toBe('boolean');
    }
  });

  it('returns a fresh object each call (no shared reference)', () => {
    const a = defaultTimelineConfig();
    const b = defaultTimelineConfig();
    a.show_agents = true;
    expect(b.show_agents).toBe(false);
  });

  it('field names match the Rust TimelineConfig snake_case contract', () => {
    const cfg = defaultTimelineConfig();
    // All 7 field names must be present and exactly these strings so the
    // serde round-trip (config_save) does not silently drop them.
    expect(Object.keys(cfg).sort()).toEqual([
      'show_agents',
      'show_commands',
      'show_errors',
      'show_fs',
      'show_hooks',
      'show_llm_cost',
      'show_mcp',
    ]);
  });

  it('spreading over a partial config restores CORE defaults for absent fields', () => {
    // Simulates old-config degradation path in snapshotIntoEditState.
    const partial = { show_agents: true };
    const merged = { ...defaultTimelineConfig(), ...partial };
    // CORE fields intact
    expect(merged.show_commands).toBe(true);
    expect(merged.show_errors).toBe(true);
    // opt-in field from partial wins
    expect(merged.show_agents).toBe(true);
    // other opt-ins still OFF
    expect(merged.show_hooks).toBe(false);
    expect(merged.show_fs).toBe(false);
    expect(merged.show_llm_cost).toBe(false);
    expect(merged.show_mcp).toBe(false);
  });
});
