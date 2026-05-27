<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { llmModels } from './llmModels.svelte';
  import { popouts } from './popouts.svelte';

  interface Props {
    popoutId: number;
  }

  let { popoutId }: Props = $props();

  interface ChatMessage {
    role: 'user' | 'assistant';
    content: string;
    model?: string;
    tokens_in?: number;
    tokens_out?: number;
    latency_ms?: number;
  }

  let messages = $state<ChatMessage[]>([]);
  let inputText = $state('');
  let sending = $state(false);
  let error = $state('');
  let messagesEl: HTMLDivElement = $state(undefined!);

  let activeModel = $derived(
    llmModels.activeModelId
      ? llmModels.getModel(llmModels.activeModelId)
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

    if (!activeModel) {
      error = 'Select a model first (Ctrl+Shift+M)';
      return;
    }

    error = '';
    messages = [...messages, { role: 'user', content: text }];
    inputText = '';
    sending = true;
    scrollToBottom();

    try {
      const result = await invoke<{
        content: string;
        tokens_in: number;
        tokens_out: number;
        model_used: string;
        latency_ms: number;
      }>('llm_complete', {
        modelId: activeModel.id,
        prompt: text,
      });

      messages = [
        ...messages,
        {
          role: 'assistant',
          content: result.content,
          model: result.model_used,
          tokens_in: result.tokens_in,
          tokens_out: result.tokens_out,
          latency_ms: result.latency_ms,
        },
      ];
    } catch (err) {
      error = String(err);
      messages = [
        ...messages,
        { role: 'assistant', content: `Error: ${err}` },
      ];
    } finally {
      sending = false;
      scrollToBottom();
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
    if (e.key === 'Escape') {
      e.stopPropagation();
      popouts.dismiss(popoutId);
    }
  }

</script>

<div class="llm-chat">
  <div class="chat-header">
    <span class="model-indicator" style="color: {activeModel?.color ? `var(${activeModel.color})` : 'var(--amber-faint)'}">
      {activeModel?.short_id ?? '---'}
    </span>
    <span class="model-name">{modelLabel}</span>
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
          </div>
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
      placeholder={activeModel ? `Message ${activeModel.short_id}...` : 'Select a model first (Ctrl+Shift+M)'}
      disabled={!activeModel || sending}
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
    font-family: 'JetBrains Mono', monospace;
  }

  .chat-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid rgba(168, 120, 48, 0.2);
    font-size: 11px;
  }

  .model-indicator {
    font-weight: 700;
    font-size: 12px;
  }

  .model-name {
    color: var(--term-white, #E8E4D8);
    flex: 1;
  }

  .sending-indicator {
    color: var(--amber-bright, #FFC840);
    font-size: 10px;
    animation: pulse 1.5s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 0.5; }
    50% { opacity: 1; }
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    gap: 8px;
    opacity: 0.5;
  }

  .empty-title {
    font-size: 14px;
    color: var(--amber-bright, #FFC840);
    font-weight: 700;
  }

  .empty-hint {
    font-size: 10px;
    color: var(--amber-faint, #A87830);
  }

  .message {
    padding: 8px;
    border-radius: var(--radius-md, 4px);
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .message.user {
    background: rgba(255, 200, 64, 0.08);
    border-left: 2px solid var(--amber-bright, #FFC840);
  }

  .message.assistant {
    background: rgba(108, 182, 255, 0.06);
    border-left: 2px solid var(--term-blue, #6CB6FF);
  }

  .msg-role {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 4px;
    color: var(--amber-faint, #A87830);
  }

  .msg-content {
    color: var(--term-white, #E8E4D8);
  }

  .msg-meta {
    font-size: 9px;
    color: var(--amber-faint, #A87830);
    margin-top: 4px;
    opacity: 0.7;
  }

  .error-bar {
    padding: 4px 12px;
    font-size: 10px;
    color: var(--term-red, #FF4848);
    background: rgba(255, 72, 72, 0.08);
    border-top: 1px solid rgba(255, 72, 72, 0.2);
  }

  .input-area {
    display: flex;
    gap: 8px;
    padding: 8px 12px;
    border-top: 1px solid rgba(168, 120, 48, 0.2);
  }

  .input-area textarea {
    flex: 1;
    background: rgba(0, 0, 0, 0.4);
    border: 1px solid rgba(168, 120, 48, 0.25);
    border-radius: var(--radius-md, 4px);
    color: var(--term-white, #E8E4D8);
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    padding: 6px 8px;
    resize: none;
    line-height: 1.4;
  }

  .input-area textarea:focus {
    border-color: var(--amber-faint, #A87830);
    outline: none;
  }

  .input-area textarea:disabled {
    opacity: 0.4;
  }

  .send-btn {
    align-self: flex-end;
    min-width: 60px;
  }
</style>
