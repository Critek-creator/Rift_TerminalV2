<script lang="ts">
  // TreeContextMenu — right-click quick actions for an fs cockpit tree node.
  //
  // Surfaces actions that already exist as Rift capabilities but previously had
  // no explicit affordance (only the undiscoverable drag gesture): inject path,
  // open in Viewer, cd here, copy paths, reveal in OS, copy vault id. Every
  // action reuses existing plumbing (terminalInject / popouts / fs_reveal).
  //
  // A `position: fixed` floating surface (clamped to the viewport), dismissed on
  // item click, Escape, outside click, or another right-click.

  import { invoke } from '@tauri-apps/api/core';
  import { popouts } from './popouts.svelte';
  import { injectIntoActiveTerminal } from './terminalInject';
  import type { TreeNode } from './Tree.svelte';
  import type { EnrichmentEntry } from './enrichmentStore.svelte';

  interface Props {
    node: TreeNode;
    x: number;
    y: number;
    enrichments: EnrichmentEntry[] | undefined;
    onClose: () => void;
  }
  let { node, x, y, enrichments, onClose }: Props = $props();

  // Resolve the absolute project root once for "Copy absolute path" — guards the
  // async race the blueprint flagged by disabling that item until it loads.
  let projectRoot = $state<string | null>(null);
  $effect(() => {
    let cancelled = false;
    invoke<string>('project_root_get')
      .then((r) => { if (!cancelled) projectRoot = r; })
      .catch(() => {});
    return () => { cancelled = true; };
  });

  // Clamp the menu inside the viewport (right/bottom edges).
  const MENU_W = 220;
  const MENU_H_EST = 280;
  const px = $derived(Math.max(4, Math.min(x, window.innerWidth - MENU_W - 8)));
  const py = $derived(Math.max(4, Math.min(y, window.innerHeight - MENU_H_EST - 8)));

  const indexVaultId = $derived(
    enrichments?.find((e) => e.provider_id === 'index' && e.vault_id)?.vault_id,
  );

  function quote(p: string): string {
    return p.includes(' ') ? `"${p}"` : p;
  }
  function dirOf(n: TreeNode): string {
    return n.isDir ? n.path : n.path.split('/').slice(0, -1).join('/');
  }
  async function copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* clipboard permission denied — silent no-op */
    }
  }

  function injectPath(): void {
    injectIntoActiveTerminal(quote(node.path));
    onClose();
  }
  function openViewer(): void {
    popouts.summon({ content: { kind: 'viewer', path: node.path }, width: 'min(1024px, 90vw)' });
    onClose();
  }
  function cdHere(): void {
    const d = dirOf(node);
    injectIntoActiveTerminal(`cd ${quote(d || '.')}`);
    onClose();
  }
  function copyRel(): void {
    void copy(node.path);
    onClose();
  }
  function copyAbs(): void {
    if (projectRoot) void copy(`${projectRoot.replace(/\/$/, '')}/${node.path}`);
    onClose();
  }
  function copyVaultId(): void {
    if (indexVaultId) void copy(indexVaultId);
    onClose();
  }
  function reveal(): void {
    void invoke('fs_reveal', { path: node.path }).catch(() => {});
    onClose();
  }
</script>

<svelte:window
  onkeydown={(e) => { if (e.key === 'Escape') onClose(); }}
  onclick={onClose}
  oncontextmenu={onClose}
/>

<div
  class="tree-context-menu"
  style="left: {px}px; top: {py}px;"
  role="menu"
  tabindex="-1"
  aria-label="File actions for {node.name}"
  onclick={(e) => e.stopPropagation()}
  onkeydown={(e) => e.stopPropagation()}
  oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); }}
>
  <button class="tcm-item" role="menuitem" onclick={injectPath}>Inject path into terminal</button>
  {#if !node.isDir}
    <button class="tcm-item" role="menuitem" onclick={openViewer}>Open in Viewer</button>
  {/if}
  <button class="tcm-item" role="menuitem" onclick={cdHere}>cd terminal here</button>
  <div class="tcm-sep" aria-hidden="true"></div>
  <button class="tcm-item" role="menuitem" onclick={copyRel}>Copy relative path</button>
  <button class="tcm-item" role="menuitem" onclick={copyAbs} disabled={!projectRoot}>
    Copy absolute path
  </button>
  {#if indexVaultId}
    <button class="tcm-item" role="menuitem" onclick={copyVaultId}>
      Copy vault id ({indexVaultId})
    </button>
  {/if}
  <div class="tcm-sep" aria-hidden="true"></div>
  <button class="tcm-item" role="menuitem" onclick={reveal}>Reveal in file manager</button>
</div>

<style>
  .tree-context-menu {
    position: fixed;
    z-index: 5000;
    min-width: 200px;
    padding: 4px;
    background: var(--bg-elevated, rgba(15, 12, 6, 0.97));
    background-image: var(--grain);
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-md);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.55), 0 0 10px rgba(255, 168, 38, 0.12);
    font-family: var(--font-family);
    display: flex;
    flex-direction: column;
    gap: 1px;
    user-select: none;
  }
  .tcm-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 5px 10px;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--amber-warm, var(--term-white));
    font-family: var(--font-family);
    font-size: var(--text-xs);
    line-height: 1.4;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--duration-fast, 120ms) var(--ease-out, ease);
  }
  .tcm-item:hover:not(:disabled) { background: rgba(255, 168, 38, 0.1); }
  .tcm-item:focus-visible { outline: 1px solid var(--amber-bright); outline-offset: -1px; }
  .tcm-item:disabled { opacity: 0.4; cursor: default; }
  .tcm-sep {
    height: 1px;
    margin: 3px 4px;
    background: var(--amber-faint, rgba(168, 120, 48, 0.3));
  }
</style>
