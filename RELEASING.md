# RELEASING

Release process for Rift Terminal. Read before cutting any tag.

---

## 1. Pre-release one-time setup

These steps are done **once** per machine / repo. Skip if already done.

### 1a. Generate updater keypair

```sh
tauri signer generate -w ~/.tauri/rift.key
```

This prints a public key string to stdout and writes the private key to
`~/.tauri/rift.key`. Keep the private key file — it is never committed.

### 1b. Wire the public key into config

In `src-tauri/tauri.conf.json`, replace the placeholder:

```json
"pubkey": "PLACEHOLDER_PUBKEY_RUN_TAURI_SIGNER_GENERATE"
```

with the public key string printed by the previous command. Commit the change.

### 1c. Add the private key as a GitHub Secret

In the repo: **Settings → Secrets and variables → Actions → New repository secret**

| Secret name                        | Value                                      |
|------------------------------------|--------------------------------------------|
| `TAURI_SIGNING_PRIVATE_KEY`        | Content of `~/.tauri/rift.key`             |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password chosen during keygen (blank = empty string) |

### 1d. (Optional) Windows code-signing

If you have a code-signing certificate (.pfx), add two more secrets:

| Secret name                    | Value                              |
|--------------------------------|------------------------------------|
| `WINDOWS_CERTIFICATE_BASE64`   | `base64 -w0 your_cert.pfx`        |
| `WINDOWS_CERTIFICATE_PASSWORD` | PFX password                       |

The release workflow ships an **unsigned MSI** if these secrets are absent —
that is intentional and acceptable for early releases.

---

## 2. Cutting a release

```sh
git tag v0.1.0
git push origin v0.1.0
```

The `release.yml` workflow triggers automatically. It:

1. Builds installers on `windows-latest` and `ubuntu-latest`.
2. Signs the updater artifacts with `TAURI_SIGNING_PRIVATE_KEY` (if set).
3. Signs the Windows MSI with the certificate (if set).
4. Uploads all bundles as a **draft** GitHub Release named `Rift Terminal v0.1.0`.

---

## 3. Publishing the draft

Visit: https://github.com/Critek-creator/Rift_TerminalV2/releases

Find the draft created by the workflow. Review the attached assets, edit the
release notes if needed, then click **Publish release**.

The published release makes `latest.json` available at the endpoint configured
in `tauri.conf.json` — the running app will poll this once the updater plugin
is wired in (see §4 and DEFERRED.md D-013).

---

## 4. Auto-update — already active (D-013 / C-018, closed 2026-04-29)

`plugins.updater.active = true` in `tauri.conf.json`. The app calls
`check()` from `@tauri-apps/plugin-updater` on session start, surfaces an
update banner with Install / Later / Dismiss buttons, and the updater bundle
is signed with the keypair from §1. Once a release is published from §3,
running app instances will detect it on next launch.

To disable temporarily for debugging, flip `plugins.updater.active` to
`false` in `tauri.conf.json` and ship a new release — installed apps will
stop polling once they update past it.

---

## 5. Pre-flight checklist (run before tagging)

Run this checklist before pushing the version tag. Catching a fail here
beats failing in CI after the tag exists.

```sh
# 1. Working tree clean and on main.
git status
git switch main && git pull --ff-only

# 2. Version bump consistent across the three sources.
#    Workspace Cargo.toml → src-tauri inherits via `version.workspace = true`.
#    package.json + tauri.conf.json must be edited in lockstep.
grep -nE '^version' Cargo.toml          # workspace
grep -E '"version"' package.json
grep -E '"version"' src-tauri/tauri.conf.json

# 3. The 10 canonical CI gates.
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
npm run check
bash tools/check-translator-boundary.sh
cargo build -p rift --features aegis --locked
cargo test -p rift-aegis --features private_modules --locked
cargo clippy -p rift --features aegis --locked
cargo build -p rift-mcp --locked && cargo test -p rift-mcp --locked  # D-014

# 4. Local bundle smoke test — produces an MSI under target/release/bundle/.
#    Slow (~10–20 min on first build), but proves the WiX side works.
npm run tauri build

# 5. Run the bundled binary manually to confirm it launches.
#    Path: target/release/rift.exe (Windows) or target/release/bundle/.../...
```

If any step fails, fix and re-run from the top.

---

## 6. Cutting a release (the actual tag)

After §5 is green:

```sh
# Update CHANGELOG.md with the new version's entry.
# Bump version in Cargo.toml + package.json + src-tauri/tauri.conf.json.
git add CHANGELOG.md Cargo.toml package.json src-tauri/tauri.conf.json
git commit -m "release: v0.X.Y"
git push origin main

# Tag and push — this is what triggers release.yml.
git tag v0.X.Y
git push origin v0.X.Y
```

Watch the run at https://github.com/Critek-creator/Rift_TerminalV2/actions —
it builds installers on `windows-latest` and `ubuntu-latest`, signs the
updater bundle with the secrets from §1c, and uploads everything as a
draft release.

---

## 7. Post-release smoke test

Before clicking **Publish release** on the draft:

1. **Download** the `Rift_*.msi` (Windows) or `rift_*.deb` / AppImage (Linux)
   asset from the draft release.
2. **Install** on a clean machine (or a VM / a different Windows user
   profile) — the install flow should run without errors and the app should
   launch from the Start menu.
3. **Verify version** — the StatusLine and About / Settings popout should
   show the new version string (matches the tag).
4. **Verify the auto-update path** — install an OLDER release first (one
   tag back), launch it, and confirm the update banner appears within a few
   seconds pointing at the new draft. Click Install → confirm the binary
   restarts at the new version.
5. **Verify `latest.json`** — the draft release ships `latest.json` (because
   `includeUpdaterJson: true` in `release.yml`); fetch it and confirm
   `signature` is non-empty (it would be empty if the signing secret hadn't
   wired through).

If everything checks out, click **Publish release** on the draft. The
moment it transitions from draft to published, the
`releases/latest/download/latest.json` URL — which `tauri.conf.json` is
configured to poll — starts serving the new manifest, and any running app
will see the update on its next launch.

---

## 8. Rollback / hotfix

If a release ships and turns out broken:

1. **Don't delete the GitHub Release** — that breaks `latest.json` for
   anyone who's already polled it.
2. Either:
   - Cut a hotfix release (preferred — bump patch version, follow §5–§7).
   - Or, if the prior release was healthy, edit the broken release's
     assets to drop `latest.json` so polling falls back to the older
     `releases/latest`. Coordinate with users via the README + Discord
     if you go this route.
3. Add a CHANGELOG.md note explaining what happened so the audit trail
   stays honest (RIFT_V2_VISION §7 — no silent stubbing or deferring
   applies to releases too).
