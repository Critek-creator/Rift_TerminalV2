<script lang="ts">
  import { invoke, Channel } from '@tauri-apps/api/core';
  import { llmModels } from './llmModels.svelte';
  import { popouts } from './popouts.svelte';

  interface Props {
    popoutId: number;
    /** Per-pane model override from the popout content. When set, this chat
     *  instance uses the specified model instead of the global active model. */
    modelOverride?: string;
  }

  let { popoutId, modelOverride: initialOverride }: Props = $props();

  interface ChatMessage {
    role: 'user' | 'assistant';
    content: string;
    model?: string;
    tokens_in?: number;
    tokens_out?: number;
    latency_ms?: number;
    task_type?: string;
    routing_reason?: string;
    cost_usd?: number;
    escalated?: boolean;
  }

  let messages = $state<ChatMessage[]>([]);
  let inputText = $state('');
  let sending = $state(false);
  let error = $state('');
  let messagesEl: HTMLDivElement = $state(undefined!);

  /** Local model override — set by the inline picker. Falls back to the
   *  initial override from the popout content, then to the global. */
  // svelte-ignore state_referenced_locally
  let localModelId = $state<string | undefined>(initialOverride);
  let pickerOpen = $state(false);

  /** Resolved model id: local override -> global active. */
  let resolvedModelId = $derived(localModelId ?? llmModels.activeModelId);

  let activeModel = $derived(
    resolvedModelId
      ? llmModels.getModel(resolvedModelId)
      : null,
  );

  let modelLabel = $derived(
    activeModel
      ? `${activeModel.short_id} ${activeModel.display_name}`
      : 'No model selected',
  );

  function scrollToBottom() {
    requestAnimationFrame(() => {
      if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
    });
  }

  async function send() {
    const text = inputText.trim();
    if (!text || sending) return;

    // @model tags let the router decide even without a selected model.
    // Only block if no model is selected AND no @tag is present.
    const hasTag = text.trimStart().startsWith('@');
    if (!activeModel && !hasTag) {
      error = 'Select a model or use @model tag (e.g. @local, @claude)';
      return;
    }

    error = '';
    messages = [...messages, { role: 'user', content: text }];
    inputText = '';
    sending = true;
    scrollToBottom();

    // Add an empty assistant message that grows as chunks arrive.
    const assistantIdx = messages.length;
    messages = [...messages, { role: 'assistant', content: '' }];

    try {
      type StreamChunk = { text: string; is_final: boolean; tokens_so_far: number };
      const onChunk: Channel<StreamChunk> = new Channel();

      onChunk.onmessage = (chunk: StreamChunk) => {
        // Append each token to the live assistant message.
        messages = messages.map((m, i) =>
          i === assistantIdx
            ? { ...m, content: m.content + chunk.text }
            : m,
        );
        scrollToBottom();
      };

      const result = await invoke<{
        content: string;
        tokens_in: number;
        tokens_out: number;
        model_used: string;
        latency_ms: number;
        task_type: string;
        routing_reason: string;
        was_overridden: boolean;
        cost_usd: number;
        escalated: boolean;
      }>('llm_stream', {
        modelId: hasTag ? null : (resolvedModelId ?? null),
        prompt: text,
        onChunk,
      });

      // Replace the streamed message with the authoritative final version (includes metadata).
      messages = messages.map((m, i) =>
        i === assistantIdx
          ? {
              ...m,
              content: result.content,
              model: result.model_used,
              tokens_in: result.tokens_in,
              tokens_out: result.tokens_out,
              latency_ms: result.latency_ms,
              task_type: result.task_type,
              routing_reason: result.routing_reason,
              cost_usd: result.cost_usd,
              escalated: result.escalated,
            }
          : m,
      );
    } catch (err) {
      error = String(err);
      // Append error to whatever partial content was already streamed.
      messages = messages.map((m, i) =>
        i === assistantIdx
          ? {
              ...m,
              content: m.content
                ? `${m.content}\n\nError: ${err}`
                : `Error: ${err}`,
            }
          : m,
      );
    } finally {
      sending = false;
      scrollToBottom();
    }
  }

  function pickModel(id: string) {
    localModelId = id;
    pickerOpen = false;
  }

  function togglePicker() {
    pickerOpen = !pickerOpen;
  }

  /** Close picker on outside click. */
  function onPickerBackdrop(e: MouseEvent) {
    if ((e.target as HTMLElement).classList.contains('picker-backdrop')) {
      pickerOpen = false;
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
    if (e.key === 'Escape') {
      if (pickerOpen) {
        pickerOpen = false;
        e.stopPropagation();
        return;
      }
      e.stopPropagation();
      popouts.dismiss(popoutId);
    }
  }

</script>

<div class="llm-chat">
  <div class="chat-header">
    <div class="model-selector-wrap">
      <button
        type="button"
        class="model-badge"
        style="border-color: {activeModel?.color ? `var(${activeModel.color})` : 'var(--amber-faint)'}; color: {activeModel?.color ? `var(${activeModel.color})` : 'var(--amber-faint)'}"
        onclick={togglePicker}
        title="Switch model for this pane"
      >
        {activeModel?.short_id ?? '---'}
        <span class="badge-caret">{pickerOpen ? '▴' : '▾'}</span>
      </button>
      {#if pickerOpen}
        <div class="picker-backdrop" role="presentation" onclick={onPickerBackdrop}>
          <div class="model-picker">
            {#each llmModels.availableModels as m (m.id)}
              <button
                type="button"
                class="picker-item"
                class:active={m.id === resolvedModelId}
                onclick={() => pickModel(m.id)}
              >
                <span
                  class="status-dot"
                  style="background: {llmModels.modelStatusColor(m.id)}"
                ></span>
                <span class="picker-short" style="color: {m.color ? `var(${m.color})` : 'var(--amber-faint)'}">{m.short_id}</span>
                <span class="picker-name">{m.display_name}</span>
              </button>
            {/each}
            {#if llmModels.availableModels.length === 0}
              <div class="picker-empty">No models configured</div>
            {/if}
          </div>
        </div>
      {/if}
    </div>
    <span class="model-name">{modelLabel}</span>
    {#if localModelId}
      <span class="override-badge" title="Per-pane model override active">override</span>
    {/if}
    {#if sending}
      <span class="sending-indicator">generating...</span>
    {/if}
  </div>

  <div class="messages" bind:this={messagesEl}>
    {#if messages.length === 0}
      <div class="empty-state">
        <div class="empty-title">Rift Router</div>
        <div class="empty-hint">Send a prompt to {modelLabel}. Ctrl+Shift+M to switch models.</div>
      </div>
    {/if}

    {#each messages as msg}
      <div class="message" class:user={msg.role === 'user'} class:assistant={msg.role === 'assistant'}>
        <div class="msg-role">{msg.role === 'user' ? 'YOU' : msg.model ?? 'MODEL'}</div>
        <div class="msg-content">{msg.content}</div>
        {#if msg.tokens_in != null}
          <div class="msg-meta">
            {msg.tokens_in} in / {msg.tokens_out} out
            {#if msg.latency_ms}| {msg.latency_ms}ms{/if}
            {#if msg.cost_usd}| ${msg.cost_usd < 0.01 ? msg.cost_usd.toFixed(4) : msg.cost_usd.toFixed(3)}{/if}
            {#if msg.escalated}<span class="escalated-badge">escalated</span>{/if}
          </div>
          {#if msg.routing_reason}
            <div class="msg-routing">{msg.routing_reason}{#if msg.task_type} · {msg.task_type}{/if}</div>
          {/if}
        {/if}
      </div>
    {/each}
  </div>

  {#if error}
    <div class="error-bar">{error}</div>
  {/if}

  <div class="input-area">
    <textarea
      bind:value={inputText}
      placeholder={activeModel ? `Message ${activeModel.short_id}... (or @model tag)` : 'Type @model to route, or select a model (Ctrl+Shift+M)'}
      disabled={sending}
      onkeydown={onKeydown}
      rows={2}
    ></textarea>
    <button
      type="button"
      class="rift-btn primary send-btn"
      disabled={!activeModel || sending || !inputText.trim()}
      onclick={send}
    >{sending ? '...' : 'Send'}</button>
  </div>
</div>

<style>
  .llm-chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 300px;
    font-family: var(--font-family);
  }

  .chat-header {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-8) var(--space-12);
    border-bottom: 1px solid var(--border-subtle);
    font-size: var(--text-sm);
  }

  .model-selector-wrap {
    position: relative;
  }

  .model-badge {
    background: transparent;
    border: 1px solid;
    border-radius: var(--radius-sm);
    padding: 2px var(--space-sm);
    font-family: var(--font-family);
    font-weight: 700;
    font-size: var(--text-sm);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    transition: background var(--duration-med), box-shadow var(--duration-med);
  }

  .model-badge:hover {
    background: var(--bg-amber-hover);
    box-shadow: var(--glow-amber-faint);
  }

  .badge-caret {
    font-size: 8px;
    opacity: 0.6;
  }

  .picker-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
  }

  .model-picker {
    position: absolute;
    top: calc(100% + var(--space-xs));
    left: 0;
    min-width: 220px;
    max-height: 240px;
    overflow-y: auto;
    background: var(--bg-elevated);
    border: 1px solid var(--border-amber-tint);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-flyout), 0 0 1px rgba(255, 168, 38, 0.15);
    z-index: 101;
    padding: var(--space-xs) 0;
    font-family: var(--font-family);
  }

  .model-picker::-webkit-scrollbar { width: 4px; }
  .model-picker::-webkit-scrollbar-track { background: transparent; }
  .model-picker::-webkit-scrollbar-thumb {
    background: var(--amber-faint);
    border-radius: var(--radius-sm);
  }

  .picker-item {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    width: 100%;
    padding: var(--space-sm) var(--space-md);
    background: transparent;
    border: none;
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-sm);
    cursor: pointer;
    text-align: left;
    transition: background var(--duration-base);
  }

  .picker-item:hover {
    background: var(--bg-amber-hover);
  }

  .picker-item.active {
    background: var(--bg-amber-selected);
  }

  .status-dot {
    width: var(--space-sm);
    height: var(--space-sm);
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }

  .picker-short {
    font-weight: 700;
    font-size: var(--text-xs);
    min-width: 36px;
  }

  .picker-name {
    color: var(--amber-faint);
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .picker-empty {
    padding: var(--space-8) var(--space-md);
    font-size: var(--text-xs);
    color: var(--amber-faint);
    opacity: 0.6;
  }

  .override-badge {
    font-size: 8px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--amber-bright);
    background: var(--bg-amber-selected);
    border: 1px solid var(--border-amber-tint);
    border-radius: var(--radius-sm);
    padding: 1px var(--space-xs);
  }

  .model-name {
    color: var(--term-white);
    flex: 1;
  }

  .sending-indicator {
    color: var(--amber-bright);
    font-size: var(--text-xs);
    animation: pulse 1.5s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 0.5; }
    50% { opacity: 1; }
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-8) var(--space-12);
    display: flex;
    flex-direction: column;
    gap: var(--space-8);
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    gap: var(--space-8);
    opacity: 0.5;
  }

  .empty-title {
    font-size: var(--text-lg);
    color: var(--amber-bright);
    font-weight: 700;
  }

  .empty-hint {
    font-size: var(--text-xs);
    color: var(--amber-faint);
  }

  .message {
    padding: var(--space-8);
    border-radius: var(--radius-md);
    font-size: var(--text-base);
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .message.user {
    background: var(--bg-amber-hover);
    border-left: 2px solid var(--amber-bright);
  }

  .message.assistant {
    background: rgba(108, 182, 255, 0.06);
    border-left: 2px solid var(--term-blue);
  }

  .msg-role {
    font-size: var(--text-2xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: var(--space-xs);
    color: var(--amber-faint);
  }

  .msg-content {
    color: var(--term-white);
  }

  .msg-meta {
    font-size: var(--text-2xs);
    color: var(--amber-faint);
    margin-top: var(--space-xs);
    opacity: 0.7;
  }

  .msg-routing {
    font-size: 8px;
    color: var(--amber-faint);
    opacity: 0.5;
    margin-top: 2px;
  }

  .escalated-badge {
    color: var(--term-red);
    font-weight: 700;
    margin-left: var(--space-xs);
  }

  .error-bar {
    padding: var(--space-xs) var(--space-12);
    font-size: var(--text-xs);
    color: var(--term-red);
    background: rgba(255, 72, 72, 0.08);
    border-top: 1px solid rgba(255, 72, 72, 0.2);
  }

  .input-area {
    display: flex;
    gap: var(--space-8);
    padding: var(--space-8) var(--space-12);
    border-top: 1px solid var(--border-subtle);
  }

  .input-area textarea {
    flex: 1;
    background: rgba(0, 0, 0, 0.4);
    border: 1px solid var(--border-amber-tint);
    border-radius: var(--radius-md);
    color: var(--term-white);
    font-family: var(--font-family);
    font-size: var(--text-base);
    padding: var(--space-sm) var(--space-8);
    resize: none;
    line-height: 1.4;
  }

  .input-area textarea:focus {
    border-color: var(--amber-faint);
  }
  .input-area textarea:focus-visible {
    outline: 1px solid var(--amber-warm);
    outline-offset: -2px;
  }

  .input-area textarea:disabled {
    opacity: 0.4;
  }

  .send-btn {
    align-self: flex-end;
    min-width: 60px;
  }
</style>
