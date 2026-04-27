#!/usr/bin/env bash
# check-translator-boundary.sh
#
# §9 Integration Decoupling build-time guard.
#
# Rift core must never speak directly to external systems (Claude Code, Aegis,
# MCP, HTTP clients, raw network sockets). All such interaction MUST go through
# translator modules under crates/rift-bus/src/translators/. This script
# enforces that rule at CI time by scanning every tracked Rust source file for
# a set of forbidden primitive patterns and failing the build if any appear
# outside the sanctioned allowlist.
#
# ALLOWLIST (patterns are PERMITTED in these paths):
#   crates/rift-bus/src/translators/**/*.rs  — the boundary itself
#   crates/rift-bus/src/ipc.rs               — Rift's internal IPC transport
#                                              (UDS/named-pipe between rift-cli
#                                              and rift-tauri; NOT an external
#                                              system)
#   **/tests/**/*.rs                         — out-of-line integration tests
#                                              (mock servers, test harnesses)
#
# FORBIDDEN PATTERNS (applied per line, as extended regex):
#   tokio::net::           — direct async network primitives
#   reqwest::              — HTTP client (preemptive; no dep today)
#   claude_(api|code|sdk|cli)::  — future Claude API direct calls
#   mcp_(client|server|core)::   — future MCP SDK direct calls
#
# INLINE #[cfg(test)] BLOCKS: this script does NOT distinguish inline test
# blocks from production code inside the same .rs file. If a production file
# legitimately needs tokio::net::* only in a test block, extract that test into
# an out-of-line integration test at crates/<name>/tests/<name>.rs — that path
# falls under the **/tests/**/*.rs allowlist and will be skipped cleanly.
#
# VIOLATION FIX: move the offending call into a new translator module at
#   crates/rift-bus/src/translators/<your-name>.rs
# and publish/consume via the bus envelope shape.
#
# Usage:
#   bash tools/check-translator-boundary.sh          # normal CI scan
#   bash tools/check-translator-boundary.sh --help   # print usage
#   bash tools/check-translator-boundary.sh --test   # self-test (exit 0 = ok)
#
# Exit codes: 0 = clean, 1 = violations found (or self-test failed).

set -euo pipefail

# ---------------------------------------------------------------------------
# Forbidden patterns (extended regex, one per line)
# ---------------------------------------------------------------------------
FORBIDDEN_PATTERNS=(
    'tokio::net::'
    'reqwest::'
    'claude_(api|code|sdk|cli)::'
    'mcp_(client|server|core)::'
)

# ---------------------------------------------------------------------------
# Help
# ---------------------------------------------------------------------------
print_help() {
    cat <<'EOF'
USAGE
  bash tools/check-translator-boundary.sh [--help | --test]

DESCRIPTION
  Scans all tracked Rust source files under crates/ and src-tauri/src/ for
  forbidden external-system primitives that must not appear outside the
  designated translator boundary (§9 Integration Decoupling).

FORBIDDEN PATTERNS
  tokio::net::                — direct async network primitives
  reqwest::                   — HTTP client
  claude_(api|code|sdk|cli):: — Claude API / SDK direct calls
  mcp_(client|server|core)::  — MCP SDK direct calls

ALLOWLIST (patterns PERMITTED in these paths)
  crates/rift-bus/src/translators/**/*.rs  — the translator boundary itself
  crates/rift-bus/src/ipc.rs               — Rift internal IPC transport
  **/tests/**/*.rs                         — out-of-line integration tests

OUTPUT ON VIOLATION
  [translator-boundary] FORBIDDEN: <pattern> at <file>:<line>: <text>
  (printed to stderr; exit code 1)

VIOLATION FIX
  Move the offending call into a new translator:
    crates/rift-bus/src/translators/<your-name>.rs
  Publish and consume it via the bus envelope shape.

MODES
  (default)  Full scan of current HEAD. Exit 0 = clean, 1 = violations.
  --test     Self-test: verifies baseline is clean, then injects a known
             violation and confirms it is caught. Exit 0 = both checks pass.
  --help     Print this message and exit 0.
EOF
}

# ---------------------------------------------------------------------------
# is_allowlisted <relative-path>
# Returns 0 (true) if the file is in the allowlist; 1 otherwise.
# ---------------------------------------------------------------------------
is_allowlisted() {
    local file="$1"
    case "$file" in
        # Out-of-line test directories (any depth under tests/).
        */tests/*.rs)     return 0 ;;
        */tests/*/*.rs)   return 0 ;;
        */tests/*/*/*.rs) return 0 ;;
        # Translator boundary — the permitted zone.
        crates/rift-bus/src/translators/*.rs)   return 0 ;;
        crates/rift-bus/src/translators/*/*.rs) return 0 ;;
        # Rift internal IPC transport (not an external system).
        crates/rift-bus/src/ipc.rs) return 0 ;;
    esac
    return 1
}

# ---------------------------------------------------------------------------
# build_file_list [extra_file ...]
# Prints one relative file path per line: tracked *.rs files in scope,
# plus any extra paths passed as arguments (for --test injection).
# ---------------------------------------------------------------------------
build_file_list() {
    {
        git ls-files '*.rs' | grep -E '^(crates/|src-tauri/src/)'
        for f in "$@"; do
            printf '%s\n' "$f"
        done
    } | sort -u
}

# ---------------------------------------------------------------------------
# run_scan [extra_file ...]
# Scans git-tracked *.rs files (plus any extras) for forbidden patterns.
# Violations are written to a caller-supplied temp file $SCAN_TMPOUT.
# Returns exit 0 if clean, 1 if any violations found.
#
# NOTE: caller must set SCAN_TMPOUT to a writable temp file path before
# calling run_scan. This avoids the bash-subshell-pipe problem where a
# variable incremented inside a `cmd | while read` loop is lost.
# ---------------------------------------------------------------------------
run_scan() {
    # SCAN_TMPOUT must be set by the caller.
    : "${SCAN_TMPOUT:?SCAN_TMPOUT not set}"
    # Truncate / create.
    : > "$SCAN_TMPOUT"

    while IFS= read -r file; do
        if is_allowlisted "$file"; then
            continue
        fi
        for pattern in "${FORBIDDEN_PATTERNS[@]}"; do
            while IFS= read -r match; do
                lineno="${match%%:*}"
                linetext="${match#*:}"
                # Trim leading whitespace.
                linetext="${linetext#"${linetext%%[![:space:]]*}"}"
                printf '[translator-boundary] FORBIDDEN: %s at %s:%s: %s\n' \
                    "$pattern" "$file" "$lineno" "$linetext" >> "$SCAN_TMPOUT"
            done < <(grep -nE "$pattern" "$file" 2>/dev/null || true)
        done
    done < <(build_file_list "$@")

    local violations
    violations=$(wc -l < "$SCAN_TMPOUT" | tr -d '[:space:]')

    if [ "$violations" -gt 0 ]; then
        cat "$SCAN_TMPOUT" >&2
        printf '[translator-boundary] %d violation(s) found. Fix: move forbidden calls into crates/rift-bus/src/translators/<name>.rs\n' \
            "$violations" >&2
        return 1
    fi

    return 0
}

# ---------------------------------------------------------------------------
# Self-test mode
# ---------------------------------------------------------------------------
run_self_test() {
    local self_test_file="crates/rift-cli/src/_boundary_self_test.rs"
    local failed=0
    local tmpout
    tmpout=$(mktemp)

    # Ensure cleanup even on error: both the scan tmpout and the injected file.
    trap 'rm -f "$self_test_file" "$tmpout"' EXIT

    # Step 1: baseline must be clean.
    printf '[translator-boundary --test] Step 1: baseline scan (expect exit 0)...\n'
    SCAN_TMPOUT="$tmpout"
    if ! run_scan; then
        printf '[translator-boundary --test] FAIL: baseline not clean — fix existing violations first\n' >&2
        failed=1
    else
        printf '[translator-boundary --test] Step 1 PASS: baseline clean.\n'
    fi

    # Step 2: inject a known violation.
    printf '[translator-boundary --test] Step 2: injecting violation into %s...\n' "$self_test_file"
    cat > "$self_test_file" <<'RUST'
// SELF-TEST: this file should be removed by check-translator-boundary.sh --test
#[allow(dead_code)]
pub fn _violation() {
    let _ = tokio::net::TcpListener::bind("0.0.0.0:0");
}
RUST

    # Step 3: re-scan with the injected file explicitly included (it is
    # untracked, so git ls-files won't see it; we pass it as an extra arg).
    printf '[translator-boundary --test] Step 3: re-scan with injected file (expect exit 1)...\n'
    SCAN_TMPOUT="$tmpout"
    if run_scan "$self_test_file"; then
        printf '[translator-boundary --test] FAIL: scan returned 0 after injecting violation\n' >&2
        failed=1
    else
        # Confirm the flagged output mentions the self-test file.
        if grep -qF "_boundary_self_test.rs" "$tmpout"; then
            printf '[translator-boundary --test] Step 3 PASS: violation correctly detected.\n'
            grep "_boundary_self_test.rs" "$tmpout" >&2
        else
            printf '[translator-boundary --test] FAIL: scan exited 1 but _boundary_self_test.rs not in output\n' >&2
            cat "$tmpout" >&2
            failed=1
        fi
    fi

    # Step 4: cleanup.
    rm -f "$self_test_file" "$tmpout"
    trap - EXIT
    printf '[translator-boundary --test] Step 4: temp file removed.\n'

    if [ "$failed" -eq 0 ]; then
        printf '[translator-boundary --test] ALL CHECKS PASSED.\n'
        return 0
    else
        printf '[translator-boundary --test] SELF-TEST FAILED.\n' >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# check_rift_aegis_private_files_ignored
#
# D-011 close (DEFERRED.md C-014): the rift-aegis crate ships a public stub
# (crates/rift-aegis/Cargo.toml + crates/rift-aegis/src/lib.rs are TRACKED).
# The real implementation lives in additional .rs files alongside lib.rs
# (detect.rs, snapshot.rs, etc.) that MUST remain gitignored. If any of
# those private files exist on disk but show up under `git ls-files`, a
# developer accidentally committed private code — fail loudly.
#
# Skip cleanly when the private files don't exist locally (public clone).
# ---------------------------------------------------------------------------
check_rift_aegis_private_files_ignored() {
    local private_files
    private_files=$(git ls-files 'crates/rift-aegis/src/*.rs' 2>/dev/null \
        | grep -v '^crates/rift-aegis/src/lib\.rs$' || true)
    if [ -n "$private_files" ]; then
        printf '[translator-boundary] FORBIDDEN: rift-aegis private impl files are tracked in git (D-011 close — only Cargo.toml + src/lib.rs may be tracked):\n' >&2
        printf '%s\n' "$private_files" | sed 's/^/  /' >&2
        return 1
    fi
    return 0
}

# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------
case "${1:-}" in
    --help)
        print_help
        exit 0
        ;;
    --test)
        run_self_test
        exit $?
        ;;
    "")
        # Normal CI mode: allocate tmpout, run scan, clean up.
        _ci_tmpout=$(mktemp)
        trap 'rm -f "$_ci_tmpout"' EXIT
        check_rift_aegis_private_files_ignored || exit 1
        SCAN_TMPOUT="$_ci_tmpout"
        run_scan
        _exit=$?
        rm -f "$_ci_tmpout"
        trap - EXIT
        exit "$_exit"
        ;;
    *)
        printf 'Unknown argument: %s\n' "$1" >&2
        print_help >&2
        exit 1
        ;;
esac
