import { describe, it, expect } from 'vitest';
import {
  ZONES,
  SURFACES,
  NOTIF_ZONE,
  zoneFor,
  surfacesIn,
  emptyZones,
  type ZoneId,
} from '../zones';
import { sectionCatalog } from '../sectionCatalog.svelte';

const ZONE_IDS = Object.keys(ZONES) as ZoneId[];

describe('zone model integrity', () => {
  it('every ZONES entry keys to its own descriptor id', () => {
    for (const id of ZONE_IDS) {
      expect(ZONES[id].id).toBe(id);
      expect(ZONES[id].title.length).toBeGreaterThan(0);
      expect(ZONES[id].role.length).toBeGreaterThan(0);
    }
  });

  it('every surface claims a valid zone', () => {
    for (const s of SURFACES) {
      expect(ZONE_IDS).toContain(s.zone);
      expect(s.id.length).toBeGreaterThan(0);
      expect(s.component.length).toBeGreaterThan(0);
    }
  });

  it('surface ids are unique (no surface registered twice)', () => {
    const ids = SURFACES.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  // N2's core invariant: the layout IS the IA. No zone may be empty — every
  // declared zone must have at least one surface, or it is dead structure.
  it('no zone is empty — every zone has a home surface', () => {
    expect(emptyZones()).toEqual([]);
    for (const id of ZONE_IDS) {
      expect(surfacesIn(id).length).toBeGreaterThan(0);
    }
  });
});

describe('zone helpers', () => {
  it('zoneFor resolves a known surface and is undefined for an orphan', () => {
    expect(zoneFor('status-line')).toBe('status');
    expect(zoneFor('terminal-grid')).toBe('terminal');
    expect(zoneFor('not-a-real-surface')).toBeUndefined();
  });

  it('surfacesIn returns only surfaces of that zone', () => {
    for (const id of ZONE_IDS) {
      for (const s of surfacesIn(id)) expect(s.zone).toBe(id);
    }
  });
});

describe('notification zone ↔ sectionCatalog', () => {
  it('NOTIF_ZONE is a real zone with a claiming surface', () => {
    expect(ZONE_IDS).toContain(NOTIF_ZONE);
    expect(surfacesIn(NOTIF_ZONE).length).toBeGreaterThan(0);
  });

  // Every dynamically-registered notif tab lives in the notification zone by
  // rule. This guards that the catalog is non-empty (so the zone is genuinely
  // populated) and that the rule's target zone stays valid.
  it('the notif-tab catalog is non-empty and bound to one zone', () => {
    const tabs = sectionCatalog.allTabs;
    expect(tabs.length).toBeGreaterThan(0);
    expect(NOTIF_ZONE).toBe('notification');
  });

  // Regression guard documenting the orphan resolution: the two tabs the IA
  // audit flagged as "undocumented orphans" are present and grouped, so they
  // are no longer floating outside the zone model.
  it('formerly-orphan tabs (integrations, feature-pipeline) are registered + grouped', () => {
    for (const id of ['integrations', 'feature-pipeline']) {
      const desc = sectionCatalog.get(id);
      expect(desc, `tab "${id}" should be registered`).toBeDefined();
      expect(desc?.group, `tab "${id}" should claim a notif group`).toBeTruthy();
    }
  });
});
