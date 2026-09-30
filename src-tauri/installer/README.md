# Windows NSIS installer template

`installer.nsi` is a copy of the official Tauri NSIS template, pinned to the
Tauri CLI that builds this app, with a small local patch.

## Source (must match the Tauri CLI)

- Tauri CLI (`@tauri-apps/cli` / `tauri-cli`): **2.11.4**
- Upstream file: `crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi`
- Tag: `tauri-cli-v2.11.4`
- Raw URL:
  `https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi`
- Baseline SHA-256:
  `20f4ecc730defb71f1342eaeaec4021df13be3d843abba0effe88ea5835fa079`

`utils.nsh`, `FileAssociation.nsh` and the language files are **not** copied
here: the bundler always writes its own copies next to the rendered template.

## When upgrading Tauri

Re-copy the upstream `installer.nsi` for the new `tauri-cli` tag and re-apply
the patch below, then verify with makensis (see below). Do not carry the patch
forward across unrelated upstream rewrites without re-reading it.

## Local patch

Only the Windows install-time reinstall flow is changed; the MSI/WiX build and
the app itself are untouched.

1. `Var AutoReinstall`, `Var UninstallFailed` — new state.
2. `PageReinstall` — keeps the WiX detection and `SemverCompare`, but no longer
   builds the "Already Installed" radio dialog. It decides (`$AutoReinstall`)
   and always `Abort`s the page. When `ALLOWDOWNGRADES` is not `true` and the
   installer is older than the installed version, it shows an error and quits.
3. `ReinstallUninstallPrevious` (new) — the upstream `PageLeaveReinstall`
   uninstall body extracted into a function, reusing the upstream
   `UninstallString` / install-mode / quoted-path handling. Difference: the old
   uninstaller is always invoked with `/P` (passive), so the upgrade is
   unattended and its "Delete application data" checkbox is never shown. Sets
   `$UninstallFailed` when the uninstaller fails or the old main binary is
   still present.
4. `PageLeaveReinstall` — now calls that function and keeps its upstream retry
   behaviour (it is only reachable if the page is ever shown again).
5. `Section EarlyChecks` — runs the old uninstaller before `Section Install`
   copies any file. On failure it shows a clear error, sets exit code 3 and
   aborts, so a failed uninstall never leaves a half-upgraded installation.

`productName` and `identifier` are deliberately unchanged: both take part in
the NSIS upgrade identity (`UNINSTKEY`, `$INSTDIR`), so changing them would
make Windows treat the new setup as a different product and create a second
Apps & Features entry.

## Compile check

The script can be syntax-checked on Linux without the app payload:

```bash
# NSIS 3.11 (the version Tauri 2.11.4 bundles)
# Render the handlebars placeholders first (the CI build does this), then:
makensis -INPUTCHARSET UTF8 installer.nsi
```

It must compile with no warnings or errors for both `allowDowngrades: true`
and `allowDowngrades: false`. Runtime behaviour (registry detection, running
app kill, Apps & Features entry) can only be verified on Windows.
