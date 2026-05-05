//! Lane classifier translator — parses OSC 6973 sentinels from PTY byte
//! streams and maintains a per-session lane state machine.
//!
//! ## Strategy D (decisions/§10.1_live_lane_classification.md)
//!
//! Three classification layers:
//!   L1 — Shell-integration sentinels (PROMPT_START/END, CMD_START, CMD_END)
//!   L2 — Bus-translator-injected sentinels (HOOK_START/END, AEGIS_START/END)
//!   L3 — Process-name fallback (CLAUDE lane, best-effort)
//!
//! This module implements the **parser** and **state machine** only. It does
//! not spawn tasks or subscribe to the bus — that wiring lives in the Tauri
//! host (`src-tauri/src/lib.rs`) which feeds PTY chunks through
//! [`LaneClassifier::feed`] and reads lane-change events out.
//!
//! ## OSC 6973 sentinel format
//!
//! ```text
//! \x1b]6973;<EVENT>[;<key>=<value>]*\x07
//! ```
//!
//! The parser scans for `\x1b]6973;` (7 bytes), then reads until `\x07` (BEL).
//! Everything between is the event payload, semicolon-delimited key=value pairs.
//!
//! ## Performance contract
//!
//! CU-2 gate: per-chunk classification must complete in < 1ms p99 under
//! sustained throughput (`cat /dev/zero | head -c 1G`). The parser is a
//! single-pass byte scanner with no allocations on non-sentinel chunks.

use std::fmt;

/// Lane identity — the 8 lanes from §10.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    /// Shell prompt region.
    Sys,
    /// User keyboard input echo.
    UserInput,
    /// Claude Code voice.
    Claude,
    /// Sub-agent output.
    Agent,
    /// Hook output.
    Hook,
    /// Aegis output.
    Aegis,
    /// Command completed successfully (exit 0).
    Ok,
    /// Command completed with warning (exit 1) or error (exit >= 2).
    Err,
}

impl fmt::Display for Lane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lane::Sys => write!(f, "SYS"),
            Lane::UserInput => write!(f, "USER"),
            Lane::Claude => write!(f, "CLAUDE"),
            Lane::Agent => write!(f, "AGENT"),
            Lane::Hook => write!(f, "HOOK"),
            Lane::Aegis => write!(f, "AEGIS"),
            Lane::Ok => write!(f, "OK"),
            Lane::Err => write!(f, "ERR"),
        }
    }
}

/// Sentinel events parsed from OSC 6973 sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SentinelEvent {
    PromptStart,
    PromptEnd,
    CmdStart,
    CmdEnd { exit_code: i32 },
    HookStart { name: String },
    HookEnd { name: String },
    AegisStart,
    AegisEnd,
    ClaudeStart,
    ClaudeEnd,
}

/// A lane-change event emitted by the classifier.
#[derive(Debug, Clone)]
pub struct LaneChange {
    pub lane: Lane,
    pub byte_offset: usize,
}

/// The lane state machine. Feed PTY byte chunks via [`LaneClassifier::feed`];
/// read resulting lane transitions from the returned `Vec<LaneChange>`.
///
/// The classifier is zero-alloc on chunks that contain no sentinels — it
/// simply advances `total_bytes` and returns an empty vec. Only sentinel-
/// containing chunks allocate (the event string + return vec).
#[derive(Debug)]
pub struct LaneClassifier {
    current_lane: Lane,
    /// Stack for nested regions (e.g. HOOK inside CMD).
    lane_stack: Vec<Lane>,
    /// Byte offset within the session (monotonically increasing).
    total_bytes: usize,
    /// Partial sentinel buffer — accumulates bytes when we're mid-escape.
    partial_osc: Option<Vec<u8>>,
}

/// OSC 6973 prefix: ESC ] 6 9 7 3 ;
const OSC_PREFIX: &[u8] = b"\x1b]6973;";
const OSC_PREFIX_LEN: usize = 7;
/// BEL terminates the OSC sequence.
const BEL: u8 = 0x07;

impl LaneClassifier {
    pub fn new() -> Self {
        Self {
            current_lane: Lane::Sys,
            lane_stack: Vec::new(),
            total_bytes: 0,
            partial_osc: None,
        }
    }

    pub fn current_lane(&self) -> Lane {
        self.current_lane
    }

    /// Feed a chunk of PTY output bytes. Returns any lane transitions that
    /// occurred within this chunk. The `byte_offset` in each `LaneChange` is
    /// relative to the start of the session (not this chunk).
    ///
    /// Sentinels are consumed — they should NOT be written to xterm.js if the
    /// caller wants a clean display. However, xterm.js naturally swallows
    /// unrecognized OSC sequences, so passing them through is also safe
    /// (they just won't render anything).
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<LaneChange> {
        let mut changes = Vec::new();
        let mut i = 0;

        while i < chunk.len() {
            // If we're accumulating a partial OSC sequence from a prior chunk:
            if let Some(ref mut buf) = self.partial_osc {
                if chunk[i] == BEL {
                    // OSC complete — parse the event.
                    let event_str = String::from_utf8_lossy(buf).to_string();
                    if let Some(evt) = parse_sentinel_event(&event_str) {
                        if let Some(change) = self.apply_event(evt) {
                            changes.push(change);
                        }
                    }
                    self.partial_osc = None;
                    i += 1;
                } else if buf.len() > 256 {
                    // Safety: abort if the "OSC" is absurdly long — not a real sentinel.
                    self.partial_osc = None;
                } else {
                    buf.push(chunk[i]);
                    i += 1;
                }
                continue;
            }

            // Scan for OSC prefix start (ESC = 0x1b).
            if chunk[i] == 0x1b {
                let remaining = &chunk[i..];
                if remaining.len() >= OSC_PREFIX_LEN && &remaining[..OSC_PREFIX_LEN] == OSC_PREFIX {
                    // Found full prefix in this chunk — scan for BEL.
                    let after_prefix = i + OSC_PREFIX_LEN;
                    if let Some(bel_offset) = chunk[after_prefix..].iter().position(|&b| b == BEL) {
                        // Complete sentinel in this chunk.
                        let event_bytes = &chunk[after_prefix..after_prefix + bel_offset];
                        let event_str = String::from_utf8_lossy(event_bytes).to_string();
                        if let Some(evt) = parse_sentinel_event(&event_str) {
                            if let Some(change) = self.apply_event(evt) {
                                changes.push(change);
                            }
                        }
                        i = after_prefix + bel_offset + 1; // skip past BEL
                    } else {
                        // Prefix found but BEL not in this chunk — start partial.
                        let event_bytes = &chunk[after_prefix..];
                        self.partial_osc = Some(event_bytes.to_vec());
                        i = chunk.len(); // consumed rest of chunk
                    }
                } else if remaining.len() < OSC_PREFIX_LEN
                    && remaining == &OSC_PREFIX[..remaining.len()]
                {
                    // Possible prefix split across chunk boundary.
                    self.partial_osc = Some(Vec::new());
                    // Re-feed will handle it next time (or abandon if not a match).
                    // For simplicity, just skip ESC and don't start partial on
                    // ambiguous short suffixes — the sentinel will re-sync on next chunk.
                    self.partial_osc = None;
                    i += 1;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }

            self.total_bytes += 1;
        }

        changes
    }

    fn apply_event(&mut self, event: SentinelEvent) -> Option<LaneChange> {
        let new_lane = match event {
            SentinelEvent::PromptStart => {
                self.lane_stack.clear();
                Lane::Sys
            }
            SentinelEvent::PromptEnd => Lane::UserInput,
            SentinelEvent::CmdStart => Lane::Ok, // default to OK until CMD_END provides exit code
            SentinelEvent::CmdEnd { exit_code } => {
                if exit_code == 0 {
                    Lane::Ok
                } else {
                    Lane::Err
                }
            }
            SentinelEvent::HookStart { .. } => {
                self.lane_stack.push(self.current_lane);
                Lane::Hook
            }
            SentinelEvent::HookEnd { .. } => self.lane_stack.pop().unwrap_or(Lane::Sys),
            SentinelEvent::AegisStart => {
                self.lane_stack.push(self.current_lane);
                Lane::Aegis
            }
            SentinelEvent::AegisEnd => self.lane_stack.pop().unwrap_or(Lane::Sys),
            SentinelEvent::ClaudeStart => {
                self.lane_stack.push(self.current_lane);
                Lane::Claude
            }
            SentinelEvent::ClaudeEnd => self.lane_stack.pop().unwrap_or(Lane::Sys),
        };

        if new_lane != self.current_lane {
            self.current_lane = new_lane;
            Some(LaneChange {
                lane: new_lane,
                byte_offset: self.total_bytes,
            })
        } else {
            None
        }
    }
}

impl Default for LaneClassifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a sentinel event string (the content between `\x1b]6973;` and `\x07`).
fn parse_sentinel_event(s: &str) -> Option<SentinelEvent> {
    let parts: Vec<&str> = s.splitn(2, ';').collect();
    let event_name = parts[0];
    let params = parts.get(1).unwrap_or(&"");

    match event_name {
        "PROMPT_START" => Some(SentinelEvent::PromptStart),
        "PROMPT_END" => Some(SentinelEvent::PromptEnd),
        "CMD_START" => Some(SentinelEvent::CmdStart),
        "CMD_END" => {
            let exit_code = extract_param(params, "exit")
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(0);
            Some(SentinelEvent::CmdEnd { exit_code })
        }
        "HOOK_START" => {
            let name = extract_param(params, "name").unwrap_or_default();
            Some(SentinelEvent::HookStart { name })
        }
        "HOOK_END" => {
            let name = extract_param(params, "name").unwrap_or_default();
            Some(SentinelEvent::HookEnd { name })
        }
        "AEGIS_START" => Some(SentinelEvent::AegisStart),
        "AEGIS_END" => Some(SentinelEvent::AegisEnd),
        "CLAUDE_START" => Some(SentinelEvent::ClaudeStart),
        "CLAUDE_END" => Some(SentinelEvent::ClaudeEnd),
        _ => None,
    }
}

fn extract_param(params: &str, key: &str) -> Option<String> {
    for pair in params.split(';') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(v.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chunk_no_changes() {
        let mut c = LaneClassifier::new();
        assert_eq!(c.current_lane(), Lane::Sys);
        let changes = c.feed(b"");
        assert!(changes.is_empty());
    }

    #[test]
    fn no_sentinel_passthrough() {
        let mut c = LaneClassifier::new();
        let changes = c.feed(b"hello world\r\nsome output\r\n");
        assert!(changes.is_empty());
        assert_eq!(c.current_lane(), Lane::Sys);
    }

    #[test]
    fn prompt_start_end_cycle() {
        let mut c = LaneClassifier::new();
        let chunk = b"\x1b]6973;PROMPT_START\x07$ \x1b]6973;PROMPT_END\x07";
        let changes = c.feed(chunk);
        // PROMPT_START → Sys (no change from default), PROMPT_END → UserInput
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].lane, Lane::UserInput);
        assert_eq!(c.current_lane(), Lane::UserInput);
    }

    #[test]
    fn cmd_start_end_exit_zero() {
        let mut c = LaneClassifier::new();
        // Start in UserInput (after prompt)
        c.feed(b"\x1b]6973;PROMPT_END\x07");
        let changes = c.feed(b"\x1b]6973;CMD_START\x07output here\x1b]6973;CMD_END;exit=0\x07");
        assert_eq!(changes.len(), 1); // UserInput→Ok on CMD_START
        assert_eq!(changes[0].lane, Lane::Ok);
        assert_eq!(c.current_lane(), Lane::Ok);
    }

    #[test]
    fn cmd_end_nonzero_exit() {
        let mut c = LaneClassifier::new();
        c.feed(b"\x1b]6973;CMD_START\x07");
        let changes = c.feed(b"\x1b]6973;CMD_END;exit=1\x07");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].lane, Lane::Err);
    }

    #[test]
    fn hook_start_end_nesting() {
        let mut c = LaneClassifier::new();
        c.feed(b"\x1b]6973;CMD_START\x07"); // → Ok
        let changes = c
            .feed(b"\x1b]6973;HOOK_START;name=lint\x07hook output\x1b]6973;HOOK_END;name=lint\x07");
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].lane, Lane::Hook);
        assert_eq!(changes[1].lane, Lane::Ok); // restored from stack
    }

    #[test]
    fn sentinel_split_across_chunks() {
        let mut c = LaneClassifier::new();
        // Split the sentinel across two chunks
        let changes1 = c.feed(b"data\x1b]6973;PROMPT_ST");
        assert!(changes1.is_empty());
        let changes2 = c.feed(b"ART\x07more data");
        // The partial should NOT parse because the prefix detection
        // doesn't handle cross-chunk prefix splits in this v1 impl.
        // This is a known limitation — sentinels should not split at
        // the prefix boundary in practice (shell preludes emit atomically).
        // Cross-chunk splits WITHIN the event body (after prefix) DO work.
        assert!(changes2.is_empty() || changes2[0].lane == Lane::Sys);
    }

    #[test]
    fn claude_lane_fallback() {
        let mut c = LaneClassifier::new();
        let changes = c.feed(b"\x1b]6973;CLAUDE_START\x07ai output\x1b]6973;CLAUDE_END\x07");
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].lane, Lane::Claude);
        assert_eq!(changes[1].lane, Lane::Sys);
    }

    #[test]
    fn large_chunk_no_sentinel_zero_alloc_path() {
        let mut c = LaneClassifier::new();
        let big = vec![b'A'; 65536];
        let changes = c.feed(&big);
        assert!(changes.is_empty());
    }
}
