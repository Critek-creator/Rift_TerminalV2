<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getLedgerSnapshot } from './llmRouting.svelte';

  interface Props {
    activeProfile?: string | null;
  }

  let { activeProfile = null }: Props = $props();

  interface ProfileEntry {
    name: string;
    project_filter?: string | null;
  }

  let profiles = $state<ProfileEntry[]>([]);
  let open = $state(false);
  let saving = $state(false);
  // Guards profile_load/_save/_delete against concurrent invokes
  // (rapid double-click, or load-while-deleting).
  let mutating = $state(false);
  let saveInput = $state('');
  let focusedIndex = $state(-1);
  let triggerEl: HTMLButtonElement = $state(undefined!);
  let dropdownEl: HTMLDivElement = $state(undefined!);
  let errorMsg = $state('');
  let errorTimer: ReturnType<typeof setTimeout> | undefined;

  const displayName = $derived(activeProfile ?? 'default');

  async function loadProfiles() {
    try {
      profiles = await invoke<ProfileEntry[]>('profile_list');
    } catch {
      profiles = [];
    }
  }

  function toggle() {
    open = !open;
    if (open) {
      loadProfiles();
      focusedIndex = -1;
    }
  }

  function close() {
    open = false;
    saving = false;
    saveInput = '';
    focusedIndex = -1;
  }

  function showError(msg: string) {
    errorMsg = msg;
    if (errorTimer) clearTimeout(errorTimer);
    errorTimer = setTimeout(() => { errorMsg = ''; errorTimer = undefined; }, 3000);
  }

  async function selectProfile(name: string) {
    if (mutating) return;
    mutating = true;
    try {
      await invoke('profile_load', { name });
    } catch (err) {
      console.warn('profile_load failed:', err);
      showError('Failed to load profile');
      return;
    } finally {
      mutating = false;
    }
    close();
  }

  function startSave() {
    saving = true;
    saveInput = '';
  }

  async function confirmSave() {
    const name = saveInput.trim();
    if (!name || mutating) return;
    mutating = true;
    try {
      // Capture the LLM routing ledger at save time so the profile carries
      // session cost/routing context.  getLedgerSnapshot() returns null on
      // serialisation failure — the backend field is Option<String> so null
      // round-trips cleanly.
      // NOTE: restore semantics are DESIGN-DEFERRED; the snapshot is captured
      // and persisted but is not applied back to the live ledger on load until
      // the UX design for that flow is settled (see profiles.rs doc comment).
      const analyticsSnapshot = getLedgerSnapshot();
      const state = {
        tabs: [],
        cockpit_visible: true,
        cockpit_panels: [],
        notification_filters: { default_threshold: null, per_tab: {} },
        analytics_snapshot: analyticsSnapshot,
      };
      await invoke('profile_save', { name, state, projectFilter: null });
    } catch (err) {
      console.warn('profile_save failed:', err);
      showError('Failed to save profile');
      return;
    } finally {
      mutating = false;
    }
    close();
    loadProfiles();
  }

  async function deleteProfile(name: string, e: MouseEvent) {
    e.stopPropagation();
    if (mutating) return;
    mutating = true;
    try {
      await invoke('profile_delete', { name });
      profiles = profiles.filter((p) => p.name !== name);
    } catch (err) {
      console.warn('profile_delete failed:', err);
      showError('Failed to delete profile');
    } finally {
      mutating = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!open) return;

    const itemCount = profiles.length + 1; // +1 for "save current"

    switch (e.key) {
      case 'Escape':
        e.preventDefault();
        close();
        break;
      case 'ArrowDown':
        e.preventDefault();
        focusedIndex = (focusedIndex + 1) % itemCount;
        break;
      case 'ArrowUp':
        e.preventDefault();
        focusedIndex = (focusedIndex - 1 + itemCount) % itemCount;
        break;
      case 'Enter':
        e.preventDefault();
        if (saving) {
          confirmSave();
        } else if (focusedIndex >= 0 && focusedIndex < profiles.length) {
          selectProfile(profiles[focusedIndex].name);
        } else if (focusedIndex === profiles.length) {
          startSave();
        }
        break;
    }
  }

  function handleClickOutside(e: MouseEvent) {
    if (
      open &&
      triggerEl &&
      !triggerEl.contains(e.target as Node) &&
      dropdownEl &&
      !dropdownEl.contains(e.target as Node)
    ) {
      close();
    }
  }

  onMount(() => {
    document.addEventListener('click', handleClickOutside, true);
    document.addEventListener('keydown', handleKeydown);
  });

  onDestroy(() => {
    document.removeEventListener('click', handleClickOutside, true);
    document.removeEventListener('keydown', handleKeydown);
    if (errorTimer) clearTimeout(errorTimer);
  });
</script>

<div class="profile-picker">
  <button type="button"
    class="profile-trigger"
    bind:this={triggerEl}
    onclick={toggle}
    title="Switch workspace profile"
  >
    <span class="profile-icon">P</span>
    <span class="profile-name">{displayName}</span>
  </button>

  {#if open}
    <div class="profile-dropdown" bind:this={dropdownEl} role="listbox" aria-label="Workspace profiles">
      {#if errorMsg}
        <div class="dropdown-error">{errorMsg}</div>
      {/if}
      {#if profiles.length === 0 && !saving}
        <div class="dropdown-empty">No saved profiles</div>
      {/if}

      {#each profiles as profile, i}
        <div
          class="dropdown-item"
          class:focused={focusedIndex === i}
          class:active={profile.name === activeProfile}
          role="option"
          aria-selected={profile.name === activeProfile}
          tabindex="-1"
          onclick={() => selectProfile(profile.name)}
          onkeydown={(e) => { if (e.key === 'Enter') selectProfile(profile.name); }}
        >
          <span class="item-name">{profile.name}</span>
          {#if profile.project_filter}
            <span class="item-filter">{profile.project_filter}</span>
          {/if}
          {#if profile.name === activeProfile}
            <span class="item-active-dot"></span>
          {/if}
          <button type="button"
            class="item-delete"
            onclick={(e) => deleteProfile(profile.name, e)}
            disabled={mutating}
            aria-label="Delete profile {profile.name}"
            title="Delete profile"
          >×</button>
        </div>
      {/each}

      <div class="dropdown-divider"></div>

      {#if saving}
        <div class="save-input-row">
          <input
            class="save-input"
            type="text"
            placeholder="profile name"
            bind:value={saveInput}
            onkeydown={(e) => {
              if (e.key === 'Enter') confirmSave();
              if (e.key === 'Escape') { saving = false; e.stopPropagation(); }
            }}
          />
          <button type="button" class="save-confirm" onclick={confirmSave}>OK</button>
        </div>
      {:else}
        <button type="button"
          class="dropdown-item save-trigger"
          class:focused={focusedIndex === profiles.length}
          onclick={startSave}
        >
          <span class="item-name">Save current...</span>
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .profile-picker {
    position: relative;
    display: inline-flex;
  }

  .profile-trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: 2px var(--space-8);
    background: transparent;
    border: 1px solid var(--amber-faint);
    border-radius: 10px;
    color: var(--amber-dim);
    font-size: var(--text-xs);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
    cursor: pointer;
    transition: border-color var(--duration-fast) var(--ease-out),
                color var(--duration-fast) var(--ease-out);
  }

  .profile-trigger:hover {
    border-color: var(--amber-primary);
    color: var(--amber-bright);
  }
  .profile-trigger:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }

  .profile-icon {
    font-weight: 700;
    font-size: var(--text-2xs);
    color: var(--amber-faint);
  }

  .profile-name {
    max-width: 80px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .profile-dropdown {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    min-width: 180px;
    max-width: 260px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.6);
    z-index: 200;
    padding: var(--space-xs) 0;
    display: flex;
    flex-direction: column;
  }

  .dropdown-error {
    padding: var(--space-xs) var(--space-md);
    font-size: var(--text-xs);
    color: var(--term-red);
    text-align: center;
    background: var(--bg-red-notice);
    border-bottom: 1px solid rgba(255, 72, 72, 0.2);
  }

  .dropdown-empty {
    padding: var(--space-sm) var(--space-md);
    font-size: var(--text-xs);
    color: var(--amber-faint);
    text-align: center;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-md);
    background: none;
    border: none;
    color: var(--amber-warm);
    font-size: var(--text-xs);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
    cursor: pointer;
    text-align: left;
    width: 100%;
    transition: background var(--duration-fast) var(--ease-out);
  }

  .dropdown-item:hover,
  .dropdown-item.focused {
    background: var(--bg-hover);
  }

  .dropdown-item.active {
    color: var(--amber-bright);
  }

  .item-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-filter {
    font-size: var(--text-2xs);
    padding: 0 var(--space-xs);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    color: var(--amber-faint);
  }

  .item-active-dot {
    width: 5px;
    height: 5px;
    border-radius: var(--radius-full);
    background: var(--amber-bright);
    flex-shrink: 0;
  }

  .item-delete {
    opacity: 0;
    background: none;
    border: none;
    color: var(--term-red);
    font-size: var(--text-xs);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
    cursor: pointer;
    padding: 0 2px;
    flex-shrink: 0;
    transition: opacity var(--duration-fast) var(--ease-out);
  }

  .dropdown-item:hover .item-delete {
    opacity: 0.7;
  }

  .item-delete:hover {
    opacity: 1 !important;
  }

  .dropdown-divider {
    height: 1px;
    background: var(--border-subtle);
    margin: var(--space-xs) 0;
  }

  .save-trigger .item-name {
    color: var(--amber-faint);
    font-style: italic;
  }

  .save-input-row {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-xs) var(--space-sm);
  }

  .save-input {
    flex: 1;
    background: var(--bg-base);
    border: 1px solid var(--border-active);
    border-radius: var(--radius-sm);
    color: var(--amber-bright);
    font-size: var(--text-xs);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
    padding: 3px var(--space-sm);
    outline: 2px solid transparent;
  }

  .save-input::placeholder {
    color: var(--amber-faint);
    opacity: 0.6;
  }

  .save-input:focus {
    border-color: var(--amber-primary);
  }

  .save-confirm {
    background: none;
    border: 1px solid var(--amber-faint);
    border-radius: var(--radius-sm);
    color: var(--amber-dim);
    font-size: var(--text-xs);
    font-family: var(--font-family, 'JetBrains Mono', monospace);
    padding: 2px var(--space-8);
    cursor: pointer;
    transition: border-color var(--duration-fast) var(--ease-out),
                color var(--duration-fast) var(--ease-out);
  }

  .save-confirm:hover {
    border-color: var(--amber-primary);
    color: var(--amber-bright);
  }
  .save-confirm:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: 1px;
  }

  .item-delete:focus-visible {
    outline: 1px solid var(--term-red);
    outline-offset: 1px;
    opacity: 1;
  }
</style>
