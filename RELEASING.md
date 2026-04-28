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

## 4. Enabling in-app updates (D-013 — not yet active)

`plugins.updater.active` is currently `false` in `tauri.conf.json`. To enable
auto-update:

1. Complete steps 1a–1c above (keypair must be real, not placeholder).
2. Follow the four unblocking steps listed in DEFERRED.md D-013:
   - Add `tauri-plugin-updater = "2"` to `src-tauri/Cargo.toml`.
   - Wire `.plugin(tauri_plugin_updater::Builder::new().build())` in `lib.rs`.
   - Add `@tauri-apps/plugin-updater` to `package.json` + frontend check logic.
3. Flip `plugins.updater.active` to `true` in `tauri.conf.json`.
4. Cut a new release — the app will now auto-detect new versions.
