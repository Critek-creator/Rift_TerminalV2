/// <reference types="svelte" />
/// <reference types="vite/client" />

export {};

declare global {
  interface Window {
    /** Exposed for MCP pty_read tool — do not remove without updating mcp_host.rs tool_pty_read */
    __RIFT_TERM__?: import('@xterm/xterm').Terminal;
  }
}
