/// <reference types="svelte" />
/// <reference types="vite/client" />

export {};

declare global {
  interface Window {
    /** Exposed for MCP pty_read tool — do not remove without updating mcp_host.rs tool_pty_read */
    __RIFT_TERM__?: import('@xterm/xterm').Terminal;
    /** Multi-terminal map keyed by session id — supports split panes */
    __RIFT_TERMS__?: Map<number, import('@xterm/xterm').Terminal>;
  }
}
