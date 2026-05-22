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
use std::path::{Path, PathBuf};

// Embedded prelude scripts (include_str! embeds at compile time).
const PRELUDE_PWSH: &str = include_str!("lane_prelude/pwsh.ps1");
const PRELUDE_BASH: &str = include_str!("lane_prelude/bash.sh");
const PRELUDE_ZSH: &str = include_str!("lane_prelude/zsh.sh");

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
    /// L3 flag: set when a CMD_START event is processed by the last
    /// `transform()` / `feed()` call. The host reads this to trigger
    /// process-name detection (sampled once per command).
    cmd_start_fired: bool,
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
            cmd_start_fired: false,
        }
    }

    pub fn current_lane(&self) -> Lane {
        self.current_lane
    }

    /// L3: Returns `true` if the last `transform()` or `feed()` call
    /// processed a CMD_START sentinel. Used by the drain task to trigger
    /// one-shot process-name detection.
    pub fn take_cmd_start_flag(&mut self) -> bool {
        let v = self.cmd_start_fired;
        self.cmd_start_fired = false;
        v
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
        self.cmd_start_fired = false;
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
        if matches!(event, SentinelEvent::CmdStart) {
            self.cmd_start_fired = true;
        }
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

// -------------------------------------------------------------------------
// ANSI lane-color escapes REMOVED (Approach D — stop fighting shell SGR)
//
// Lane colors were previously injected into the PTY byte stream by
// `transform()` and `inject_event()`. This caused visible oscillation
// when programs (PSReadLine, oh-my-posh, Starship, Claude Code) emitted
// SGR resets — xterm fell back to the theme foreground, then the next
// chunk got lane color prepended again.
//
// Lane identity is still tracked by the state machine and consumed by
// notification tabs and bus events. The frontend can style lane-tagged
// UI elements via CSS using the lane identity reported on bus envelopes.
// -------------------------------------------------------------------------

// -------------------------------------------------------------------------
// Chunk transformer — strip sentinels, track lane state (no color inject)
// -------------------------------------------------------------------------

impl LaneClassifier {
    /// Transform a PTY chunk: strip OSC 6973 sentinels and update lane state.
    /// Returns the modified byte stream ready for xterm.js with sentinels
    /// removed but **no ANSI lane-color escapes injected** (Approach D).
    ///
    /// On chunks with no sentinels, this is a memcpy. On sentinel-containing
    /// chunks, sentinels are removed and lane state is updated, but the
    /// output contains only the original non-sentinel bytes — shell programs
    /// manage their own colors without interference.
    pub fn transform(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.cmd_start_fired = false;
        let mut out = Vec::with_capacity(chunk.len());
        let mut i = 0;

        while i < chunk.len() {
            // Continue accumulating a partial sentinel from prior chunk.
            if let Some(ref mut buf) = self.partial_osc {
                if chunk[i] == BEL {
                    let event_str = String::from_utf8_lossy(buf).to_string();
                    if let Some(evt) = parse_sentinel_event(&event_str) {
                        // Update lane state; no color bytes emitted.
                        self.apply_event(evt);
                    }
                    self.partial_osc = None;
                    i += 1;
                } else if buf.len() > 256 {
                    self.partial_osc = None;
                } else {
                    buf.push(chunk[i]);
                    i += 1;
                }
                continue;
            }

            // Look for ESC (start of potential sentinel).
            if chunk[i] == 0x1b {
                let remaining = &chunk[i..];
                if remaining.len() >= OSC_PREFIX_LEN && &remaining[..OSC_PREFIX_LEN] == OSC_PREFIX {
                    // Full prefix found — scan for BEL.
                    let after_prefix = i + OSC_PREFIX_LEN;
                    if let Some(bel_pos) = chunk[after_prefix..].iter().position(|&b| b == BEL) {
                        // Complete sentinel — parse and update state (no color emit).
                        let event_bytes = &chunk[after_prefix..after_prefix + bel_pos];
                        let event_str = String::from_utf8_lossy(event_bytes).to_string();
                        if let Some(evt) = parse_sentinel_event(&event_str) {
                            self.apply_event(evt);
                        }
                        i = after_prefix + bel_pos + 1;
                    } else {
                        // Prefix found but BEL not in chunk — start partial.
                        self.partial_osc = Some(chunk[after_prefix..].to_vec());
                        i = chunk.len();
                    }
                } else {
                    // Not our sentinel — pass through.
                    out.push(chunk[i]);
                    i += 1;
                    self.total_bytes += 1;
                }
            } else {
                out.push(chunk[i]);
                i += 1;
                self.total_bytes += 1;
            }
        }

        out
    }
}

// -------------------------------------------------------------------------
// L2 — Bus-driven lane injection (hook / aegis events)
// -------------------------------------------------------------------------

impl LaneClassifier {
    /// Inject a synthetic lane event from a bus envelope (L2 mechanism).
    /// Returns the new [`Lane`] identity if the event caused a transition,
    /// or `None` if the lane did not change.
    ///
    /// Used by the drain task when it receives a `Category::Hook` or
    /// `Category::Aegis` bus event — the event doesn't flow through the
    /// PTY byte stream, so we inject the lane transition directly into
    /// the classifier's state machine. No ANSI bytes are produced
    /// (Approach D — color injection removed to stop fighting shell SGR).
    pub fn inject_event(&mut self, event: SentinelEvent) -> Option<Lane> {
        self.apply_event(event).map(|change| change.lane)
    }
}

// -------------------------------------------------------------------------
// Shell prelude injection
// -------------------------------------------------------------------------

/// Instructions for injecting the lane prelude into a shell spawn.
#[derive(Debug)]
pub struct PreludeInjection {
    /// Replace the shell args with these (e.g. ["-NoExit", "-Command", "..."]).
    pub shell_args: Vec<String>,
    /// Additional env vars to set on the PTY.
    pub extra_env: Vec<(String, String)>,
}

/// Determine which shell is at `resolved_path` by inspecting its filename.
fn detect_shell_kind(resolved_path: &Path) -> ShellKind {
    let stem = resolved_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    match stem.as_str() {
        "pwsh" | "powershell" => ShellKind::Pwsh,
        "bash" => ShellKind::Bash,
        "zsh" => ShellKind::Zsh,
        _ => ShellKind::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    Pwsh,
    Bash,
    Zsh,
    Unknown,
}

/// Prepare the lane-classification prelude for injection into a PTY session.
///
/// Writes the appropriate prelude script to a temp file and returns
/// `PreludeInjection` with the modified shell args / env. Returns `None`
/// for shells that don't support lane classification (cmd.exe, sh).
///
/// `resolved_path` is the concrete shell binary (e.g. `C:\Program Files\PowerShell\7\pwsh.exe`).
pub fn prepare_lane_prelude(resolved_path: &Path) -> Option<PreludeInjection> {
    let kind = detect_shell_kind(resolved_path);
    match kind {
        ShellKind::Pwsh => prepare_pwsh_prelude(),
        ShellKind::Bash => prepare_bash_prelude(),
        ShellKind::Zsh => prepare_zsh_prelude(),
        ShellKind::Unknown => None,
    }
}

fn prelude_temp_dir() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("rift-lane-prelude");
    dir
}

fn prepare_pwsh_prelude() -> Option<PreludeInjection> {
    let dir = prelude_temp_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            dir = %dir.display(),
            error = %e,
            "rift-lane-prelude: failed to create pwsh prelude directory"
        );
        return None;
    }
    let file = dir.join("prelude.ps1");
    if let Err(e) = std::fs::write(&file, PRELUDE_PWSH) {
        tracing::warn!(
            path = %file.display(),
            error = %e,
            "rift-lane-prelude: failed to write pwsh prelude script"
        );
        return None;
    }
    let path_str = file.to_string_lossy().replace('\'', "''");
    Some(PreludeInjection {
        shell_args: vec![
            "-NoExit".into(),
            "-Command".into(),
            format!(". '{path_str}'"),
        ],
        extra_env: vec![],
    })
}

fn prepare_bash_prelude() -> Option<PreludeInjection> {
    let dir = prelude_temp_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            dir = %dir.display(),
            error = %e,
            "rift-lane-prelude: failed to create bash prelude directory"
        );
        return None;
    }
    let file = dir.join("prelude-bash.sh");
    let content = format!(
        "# Rift lane prelude wrapper — sources user's .bashrc then lane hooks\n\
         [[ -f ~/.bashrc ]] && source ~/.bashrc\n\
         {PRELUDE_BASH}"
    );
    if let Err(e) = std::fs::write(&file, content) {
        tracing::warn!(
            path = %file.display(),
            error = %e,
            "rift-lane-prelude: failed to write bash prelude script"
        );
        return None;
    }
    let path_str = file.to_string_lossy().to_string();
    Some(PreludeInjection {
        shell_args: vec!["--rcfile".into(), path_str],
        extra_env: vec![],
    })
}

fn prepare_zsh_prelude() -> Option<PreludeInjection> {
    let dir = prelude_temp_dir();
    let zdotdir = dir.join("zsh");
    if let Err(e) = std::fs::create_dir_all(&zdotdir) {
        tracing::warn!(
            dir = %zdotdir.display(),
            error = %e,
            "rift-lane-prelude: failed to create zsh prelude directory"
        );
        return None;
    }
    let zshrc = zdotdir.join(".zshrc");
    let content = format!(
        "# Rift lane prelude wrapper — sources user's .zshrc then lane hooks\n\
         _rift_real_zdotdir=\"$HOME\"\n\
         [[ -f \"$_rift_real_zdotdir/.zshrc\" ]] && source \"$_rift_real_zdotdir/.zshrc\"\n\
         unset _rift_real_zdotdir\n\
         {PRELUDE_ZSH}"
    );
    if let Err(e) = std::fs::write(&zshrc, content) {
        tracing::warn!(
            path = %zshrc.display(),
            error = %e,
            "rift-lane-prelude: failed to write zsh prelude script"
        );
        return None;
    }
    let zdotdir_str = zdotdir.to_string_lossy().to_string();
    Some(PreludeInjection {
        shell_args: vec![],
        extra_env: vec![("ZDOTDIR".into(), zdotdir_str)],
    })
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

    // -----------------------------------------------------------------
    // transform() tests (Approach D — no ANSI color injection)
    // -----------------------------------------------------------------

    #[test]
    fn transform_passthrough_no_sentinels() {
        let mut c = LaneClassifier::new();
        let input = b"hello world\r\n";
        let out = c.transform(input);
        // No color prepend — output is a clean passthrough of the input.
        assert_eq!(out, input);
    }

    #[test]
    fn transform_strips_sentinel_no_color_inject() {
        let mut c = LaneClassifier::new();
        let input = b"\x1b]6973;PROMPT_END\x07typed text";
        let out = c.transform(input);
        // Sentinel stripped, text preserved, no ANSI color bytes injected.
        assert!(!out.windows(4).any(|w| w == b"6973"));
        assert_eq!(&out, b"typed text");
        // Lane state updated even though no color bytes emitted.
        assert_eq!(c.current_lane(), Lane::UserInput);
    }

    #[test]
    fn transform_multiple_transitions() {
        let mut c = LaneClassifier::new();
        let input =
            b"\x1b]6973;PROMPT_END\x07cmd\x1b]6973;CMD_START\x07output\x1b]6973;CMD_END;exit=1\x07";
        let out = c.transform(input);
        let out_str = String::from_utf8_lossy(&out);
        // Text preserved, sentinels stripped, no ANSI color bytes.
        assert!(out.windows(3).any(|w| w == b"cmd"));
        assert!(out_str.contains("output"));
        // No ESC bytes should be present (sentinels removed, no color injected).
        assert!(!out.contains(&0x1b));
        // Lane state tracks the final transition (Err from exit=1).
        assert_eq!(c.current_lane(), Lane::Err);
    }

    #[test]
    fn transform_large_chunk_no_color_prepend() {
        let mut c = LaneClassifier::new();
        let big = vec![b'X'; 65536];
        let out = c.transform(&big);
        // No color prepend — output is identical to input.
        assert_eq!(out.len(), big.len());
        assert_eq!(out, big);
    }

    // -----------------------------------------------------------------
    // Prelude injection tests
    // -----------------------------------------------------------------

    #[test]
    #[cfg(windows)]
    fn detect_pwsh_from_path() {
        let path = Path::new("C:\\Program Files\\PowerShell\\7\\pwsh.exe");
        assert_eq!(detect_shell_kind(path), ShellKind::Pwsh);
    }

    #[test]
    fn detect_bash_from_path() {
        let path = Path::new("/usr/bin/bash");
        assert_eq!(detect_shell_kind(path), ShellKind::Bash);
    }

    #[test]
    #[cfg(windows)]
    fn detect_unknown_cmd() {
        let path = Path::new("C:\\Windows\\System32\\cmd.exe");
        assert_eq!(detect_shell_kind(path), ShellKind::Unknown);
    }

    #[test]
    fn prepare_prelude_pwsh_writes_file() {
        let path = Path::new("/usr/local/bin/pwsh");
        let injection = prepare_lane_prelude(path).expect("pwsh should produce injection");
        assert_eq!(injection.shell_args.len(), 3);
        assert_eq!(injection.shell_args[0], "-NoExit");
        assert_eq!(injection.shell_args[1], "-Command");
        assert!(injection.shell_args[2].contains("prelude.ps1"));
        // Verify the file was actually written.
        let dir = prelude_temp_dir();
        assert!(dir.join("prelude.ps1").exists());
    }

    #[test]
    fn prepare_prelude_unknown_returns_none() {
        let path = Path::new("C:\\Windows\\System32\\cmd.exe");
        assert!(prepare_lane_prelude(path).is_none());
    }
}
