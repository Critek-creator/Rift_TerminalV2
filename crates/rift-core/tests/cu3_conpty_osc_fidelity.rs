//! CU-3 — ConPTY OSC sequence fidelity test.
//!
//! IGNORED by default: PTY write→read round-trip does not work in the cargo
//! test harness (ConPTY output doesn't reach the reader thread — same
//! limitation documented in pty.rs `writer_accepts_bytes` test). CU-3
//! validation must be run in the live Tauri app via `npm run tauri:dev`.
//!
//! Manual validation steps:
//! 1. `npm run tauri:dev`
//! 2. In the Rift terminal, run:
//!    `powershell -NoProfile -Command "[Console]::Write([char]27+']6973;TEST'+[char]7)"`
//! 3. Check stderr/logs for the sentinel arriving in the PTY reader.
//!
//! Run (ignored): `cargo test -p rift-core --test cu3_conpty_osc_fidelity -- --ignored`

use rift_core::pty::{PtyDims, PtySession};
use std::time::Duration;
use tokio::time::timeout;

const SENTINEL_BYTES: &[u8] = b"\x1b]6973;TEST_ROUNDTRIP\x07";

#[tokio::test]
#[ignore = "PTY write→read round-trip requires live Tauri app (see module doc)"]
async fn osc_6973_survives_conpty_roundtrip() {
    let (mut output, control) = PtySession::spawn(PtyDims::default()).expect("spawn pty");

    // Wait for first output (shell prompt) — matches the existing working test.
    let first = timeout(Duration::from_secs(5), output.recv())
        .await
        .expect("should get first chunk within 5s")
        .expect("channel open");
    eprintln!("[CU-3] First chunk: {} bytes", first.len());

    // First: verify basic I/O works (echo test).
    control
        .write(b"echo RIFT_ECHO_TEST\r\n")
        .expect("write echo");
    let mut echo_buf = Vec::new();
    let echo_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < echo_deadline {
        match timeout(Duration::from_millis(300), output.recv()).await {
            Ok(Some(chunk)) => {
                echo_buf.extend_from_slice(&chunk);
                if echo_buf
                    .windows(15)
                    .any(|w| w == b"RIFT_ECHO_TEST\r" || w == b"RIFT_ECHO_TEST\n")
                {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => continue,
        }
    }
    eprintln!(
        "[CU-3] Echo test: {} bytes. Contains marker: {}",
        echo_buf.len(),
        echo_buf.windows(14).any(|w| w == b"RIFT_ECHO_TEST")
    );

    // Now emit the OSC sentinel from within the shell.
    #[cfg(windows)]
    let cmd = "powershell -NoProfile -Command \"[Console]::Write([char]27 + ']6973;TEST_ROUNDTRIP' + [char]7)\"\r\n";
    #[cfg(not(windows))]
    let cmd = "printf '\\033]6973;TEST_ROUNDTRIP\\007'\r\n";

    control.write(cmd.as_bytes()).expect("write cmd");

    // Collect output for up to 8 seconds, looking for the sentinel.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut collected = Vec::new();

    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_millis(500), output.recv()).await {
            Ok(Some(chunk)) => {
                collected.extend_from_slice(&chunk);
                if collected
                    .windows(SENTINEL_BYTES.len())
                    .any(|w| w == SENTINEL_BYTES)
                {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => continue,
        }
    }

    control.kill().ok();

    let found = collected
        .windows(SENTINEL_BYTES.len())
        .any(|w| w == SENTINEL_BYTES);

    if !found {
        let preview = String::from_utf8_lossy(&collected[..collected.len().min(800)]);
        panic!(
            "CU-3 FAILED: OSC 6973 did NOT survive PTY round-trip.\n\
             Collected {} bytes. Preview:\n{preview}",
            collected.len(),
        );
    }

    eprintln!("CU-3 PASSED: OSC 6973 survived ConPTY round-trip");
}
