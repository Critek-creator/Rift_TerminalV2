//! Commands translator — publishes `Category::Pty` / `kind="command.submitted"` envelopes.
//!
//! This translator surfaces submitted PTY input lines to the commands notification
//! tab. The backend calls [`CommandBuffer::feed`] with raw bytes from `pty_write`,
//! iterates the returned completed lines, and calls [`publish`] for each.
//!
//! ## Design notes
//!
//! ### Line-ending handling
//!
//! `feed` treats `\r`, `\n`, and `\r\n` each as a single command boundary:
//! - `\r\n` is collapsed to one boundary (not two), per terminal convention.
//! - An empty buffer flush (`\r` with nothing buffered) returns `""` — a real
//!   signal that the user pressed Enter on an empty prompt.
//!
//! ### Raw-length reporting
//!
//! `feed` returns `Vec<(String, usize)>` where the `usize` is the number of
//! **raw input bytes consumed by that command**, inclusive of the line-ending
//! byte(s). This lets the caller pass an accurate `raw_len` to [`publish`]
//! without recomputing it from the decoded string (which may differ due to
//! lossy UTF-8 replacement).
//!
//! ### Known limitation — no escape-sequence interpretation
//!
//! The buffer captures literal bytes. Backspace, cursor movement, and other
//! ANSI/VT escape sequences are decoded lossy-UTF-8 and appear as replacement
//! characters or raw escape text in the command string. v1 records what was
//! literally sent; echo-parsing PTY output to derive the "true" command is
//! deferred (requires prompt-detection design).
//!
//! ## Payload shape
//!
//! ```json
//! {
//!   "session_id": <u32>,
//!   "command":    "<UTF-8 lossy string>",
//!   "raw_len":    <usize — original byte count including line ending>
//! }
//! ```
//!
//! `kind` is always `"command.submitted"`. Adding further kinds under
//! `Category::Pty` is additive and does NOT bump `CURRENT_VERSION`
//! (per `envelope-version-additive-categories-no-bump`).

use serde_json::json;

use crate::{Category, Envelope, RiftBus};

// ---------------------------------------------------------------------------
// CommandBuffer
// ---------------------------------------------------------------------------

/// Per-session line buffer. Accumulates raw PTY-input bytes between newline
/// boundaries and emits complete command strings.
///
/// Create one per Tauri session; call [`CommandBuffer::feed`] for each
/// `pty_write` payload; iterate the returned `Vec` to publish envelopes.
///
/// Empty lines (`""`) ARE returned — an empty `\r` means the user pressed
/// Enter on a blank prompt, which is a valid event. Callers may filter if
/// desired, but this translator does not.
pub struct CommandBuffer {
    buf: Vec<u8>,
}

impl CommandBuffer {
    /// Create a fresh buffer.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed raw input bytes (whatever `pty_write` received).
    ///
    /// Returns zero or more completed command lines as `(decoded_string, raw_len)`.
    /// `raw_len` is the total raw-byte count for that command **including** the
    /// trailing line-ending (`\r`, `\n`, or `\r\n`). Trailing incomplete bytes
    /// remain buffered until the next call.
    ///
    /// `\r\n` is treated as a single boundary — it produces exactly one entry,
    /// not two. Decoding is UTF-8 lossy (`String::from_utf8_lossy`).
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<(String, usize)> {
        let mut results = Vec::new();
        let mut i = 0;

        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\r' {
                // Consume a following \n if present (\r\n → single boundary).
                let ending_len = if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    2
                } else {
                    1
                };
                let cmd = String::from_utf8_lossy(&self.buf).into_owned();
                let raw_len = self.buf.len() + ending_len;
                self.buf.clear();
                results.push((cmd, raw_len));
                i += ending_len;
            } else if b == b'\n' {
                let cmd = String::from_utf8_lossy(&self.buf).into_owned();
                let raw_len = self.buf.len() + 1;
                self.buf.clear();
                results.push((cmd, raw_len));
                i += 1;
            } else {
                self.buf.push(b);
                i += 1;
            }
        }

        results
    }
}

impl Default for CommandBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// publish
// ---------------------------------------------------------------------------

/// Publish a `Category::Pty / kind="command.submitted"` envelope.
///
/// Fire-and-forget: if the bus publish itself fails (bus closed, zero
/// capacity) the error is logged to stderr and the function returns normally.
/// Callers are already in a hot write path and must not be disrupted by a
/// secondary bus failure.
///
/// # Arguments
///
/// * `bus`        — the shared [`RiftBus`] instance.
/// * `session_id` — PTY session id (matches the id returned by `pty_start`).
/// * `command`    — the decoded command string (UTF-8 lossy).
/// * `raw_len`    — original byte count of the command bytes FROM the frontend,
///   INCLUDING the trailing line-ending character(s).
pub fn publish(bus: &RiftBus, session_id: u32, command: String, raw_len: usize) {
    let payload = json!({
        "session_id": session_id,
        "command":    command,
        "raw_len":    raw_len,
    });
    let mut env = Envelope::new(Category::Pty, "command.submitted");
    env.payload = payload;
    bus.publish(env);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RiftBus, SubscribeFilter};
    use tokio::time::{timeout, Duration};

    // --- CommandBuffer tests ---

    #[test]
    fn feed_emits_on_carriage_return() {
        let mut buf = CommandBuffer::new();
        let results = buf.feed(b"ls\r");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "ls");
        // buffer should be empty
        let next = buf.feed(b"");
        assert!(next.is_empty());
    }

    #[test]
    fn feed_emits_on_newline() {
        let mut buf = CommandBuffer::new();
        let results = buf.feed(b"pwd\n");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "pwd");
    }

    #[test]
    fn feed_handles_crlf_as_single_boundary() {
        let mut buf = CommandBuffer::new();
        let results = buf.feed(b"ls\r\n");
        // must be exactly ONE command, not two
        assert_eq!(results.len(), 1, "\\r\\n must produce exactly one command");
        assert_eq!(results[0].0, "ls");
    }

    #[test]
    fn feed_handles_multiple_in_one_call() {
        let mut buf = CommandBuffer::new();
        let results = buf.feed(b"a\rb\rc\r");
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, "a");
        assert_eq!(results[1].0, "b");
        assert_eq!(results[2].0, "c");
    }

    #[test]
    fn feed_partial_buffers() {
        let mut buf = CommandBuffer::new();
        let first = buf.feed(b"hello");
        assert!(first.is_empty(), "no boundary yet — should buffer");
        let second = buf.feed(b" world\r");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].0, "hello world");
    }

    #[test]
    fn feed_empty_command() {
        let mut buf = CommandBuffer::new();
        let results = buf.feed(b"\r");
        assert_eq!(results.len(), 1, "empty Enter is still a command");
        assert_eq!(results[0].0, "");
    }

    #[test]
    fn feed_lossy_utf8() {
        let mut buf = CommandBuffer::new();
        let results = buf.feed(b"\xFF\r");
        assert_eq!(results.len(), 1);
        // \xFF is not valid UTF-8; lossy decode replaces it with U+FFFD
        assert_eq!(results[0].0, "\u{FFFD}");
    }

    #[test]
    fn feed_raw_len_carriage_return() {
        // "ls\r" → raw_len should be 3 (2 bytes for "ls" + 1 for \r)
        let mut buf = CommandBuffer::new();
        let results = buf.feed(b"ls\r");
        assert_eq!(results[0].1, 3);
    }

    #[test]
    fn feed_raw_len_crlf() {
        // "ls\r\n" → raw_len should be 4 (2 + 2)
        let mut buf = CommandBuffer::new();
        let results = buf.feed(b"ls\r\n");
        assert_eq!(results[0].1, 4);
    }

    // --- publish / envelope-shape tests ---

    #[test]
    fn publish_envelope_shape() {
        let bus = RiftBus::default();
        publish(&bus, 7, "ls -la".into(), 8);

        let (snapshot, _sub) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot.len(), 1, "expected exactly one envelope");
        let env = &snapshot[0];
        assert_eq!(env.category, Category::Pty);
        assert_eq!(env.kind, "command.submitted");
        assert_eq!(env.payload["session_id"], 7);
        assert_eq!(env.payload["command"], "ls -la");
        assert_eq!(env.payload["raw_len"], 8);
    }

    #[tokio::test]
    async fn subscribe_roundtrip() {
        let bus = RiftBus::default();
        let (_snapshot, mut sub) = bus.subscribe(SubscribeFilter::Category(Category::Pty));

        publish(&bus, 1, "echo hello".into(), 11);

        let received = timeout(Duration::from_secs(1), sub.recv())
            .await
            .expect("recv within 1s")
            .expect("ok");

        assert_eq!(received.category, Category::Pty);
        assert_eq!(received.kind, "command.submitted");
        assert_eq!(received.payload["session_id"], 1);
        assert_eq!(received.payload["command"], "echo hello");
    }

    /// Version must not be bumped (additive kind under existing Category).
    #[test]
    fn version_not_bumped() {
        let bus = RiftBus::default();
        publish(&bus, 0, "test".into(), 5);
        let (snapshot, _) = bus.subscribe(SubscribeFilter::All);
        assert_eq!(snapshot[0].version, crate::CURRENT_VERSION);
    }
}
