# Rift Terminal — Release Runbook

Repeatable process for cutting a release. Follow every step in order.

## Pre-Release Checklist

- [ ] All CI checks passing on `main` (fmt, clippy, build, test, svelte-check, boundary check)
- [ ] No uncommitted changes (`git status` clean)
- [ ] CHANGELOG or commit history documents what's new
- [ ] `npm run tauri:dev` — app launches, no console errors

## 1. Version Bump

Bump the version in **all four locations**. Grep for the old version to catch stragglers.

```bash
# Files to update:
# 1. Cargo.toml (workspace.package.version)
# 2. src-tauri/tauri.conf.json ("version" field)
# 3. package.json ("version" field)

# Verify no old version remains:
grep -rn "OLD_VERSION" Cargo.toml src-tauri/tauri.conf.json package.json
```

Lesson: `version-bump-before-tag` — always bump BEFORE tagging. Tagging first bakes the old version into release artifacts.

## 2. Commit the Version Bump

```bash
git add Cargo.toml Cargo.lock src-tauri/tauri.conf.json package.json
git commit -m "chore: bump version to vX.Y.Z"
```

## 3. Tag and Push

```bash
git tag vX.Y.Z
git push origin main
git push origin vX.Y.Z
```

The `v*` tag push triggers the release workflow (`.github/workflows/release.yml`).

## 4. Monitor the Release Workflow

Watch the GitHub Actions run at:
`https://github.com/Critek-creator/Rift_TerminalV2/actions`

The workflow builds on three platforms:
- `windows-latest` — produces `Rift-X.Y.Z-setup.exe` (NSIS installer)
- `ubuntu-latest` — produces `Rift_X.Y.Z_amd64.AppImage`
- `macos-latest` — produces `Rift_X.Y.Z.dmg` + `Rift.app.tar.gz`

`tauri-action` creates a **draft release** with all artifacts attached.

## 5. Verify Artifacts

Download each artifact and smoke test:

### Windows
1. Run the `.exe` installer
2. Launch Rift from Start Menu
3. Verify version in Settings > About
4. Type a command in the terminal
5. Check notification tabs respond (errors tab, hooks tab)
6. Close and reopen — verify window position restores

### Linux
1. `chmod +x Rift_*.AppImage && ./Rift_*.AppImage`
2. Same smoke checks as Windows

### macOS
1. Open the `.dmg`, drag to Applications
2. Right-click > Open (bypass Gatekeeper for unsigned beta)
3. Same smoke checks as Windows

## 6. Auto-Updater Verification

If a previous release exists:
1. Install the **previous** version
2. Launch it
3. Wait for the startup update check (or click "Check for Updates" in Settings)
4. Verify the update banner appears with the new version
5. Apply the update and verify Rift restarts at the new version

The updater reads `latest.json` from the GitHub release. `tauri-action` generates this file automatically with platform-specific URLs and signatures.

## 7. Publish the Release

1. Go to the draft release on GitHub
2. Review the auto-generated release notes
3. Edit if needed — add a summary of key changes
4. Check "Set as the latest release"
5. Click "Publish release"

## 8. Post-Release

- [ ] Update the website download links (if applicable)
- [ ] Post to Patreon (if significant release)
- [ ] Update CLAUDE.md project status with the new version
- [ ] Update p006 vault `status:` line

## Troubleshooting

### Release workflow fails on one platform
The matrix uses `fail-fast: false`, so other platforms still build. Fix the failing platform and re-run the job, or delete the tag, fix, and re-tag.

### Wrong version in artifacts
Delete the GitHub release draft, delete the remote tag (`git push origin :refs/tags/vX.Y.Z`), fix the version, re-commit, re-tag, re-push.

### Auto-updater doesn't detect the new version
Check that `latest.json` exists in the release assets and contains entries for the user's platform. The updater endpoint is:
`https://github.com/Critek-creator/Rift_TerminalV2/releases/latest/download/latest.json`

### SmartScreen / Gatekeeper blocks installation
Expected for unsigned beta. Document the bypass in getting-started.md. Signing certs are a future investment.
