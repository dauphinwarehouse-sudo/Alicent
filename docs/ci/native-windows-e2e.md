# Native Windows E2E and release-audit plan

## Scope labels

Every result must use one of these exact scopes:

- **browser smoke** — Playwright opens the Vite preview or the deterministic fixture. It does not install an application, cross the Tauri IPC boundary, use WebView2 as hosted by Alicent, or prove filesystem integration.
- **installed native Windows** — the NSIS artifact is installed silently, `Alicent.exe` is launched, Playwright attaches to that process's WebView2 instance, the Tauri bridge is detected, and the development-server origin is rejected.
- **physical Windows smoke** — a named tester records the Windows edition/build and hardware. This repository has no result in this category yet.

A green GitHub-hosted `windows-2022` run is evidence for the installed native harness on Windows Server 2022. It is **not** evidence of a physical Windows 10 or Windows 11 smoke.

## Automated installed-app flow

Run after producing the unsigned NSIS bundle:

```powershell
npm run test:native:windows -- -InstallerPath target\release\bundle\nsis\Alicent_0.1.0_x64-setup.exe
```

The harness:

1. installs the exact NSIS artifact and records its SHA-256;
2. finds the installed executable from uninstall metadata or standard roots;
3. launches WebView2 with a loopback CDP endpoint;
4. verifies a Tauri bridge, rejects the Vite development origin, and checks that the native-only project action is enabled;
5. runs the automated WCAG audit in the installed WebView;
6. writes a redacted diagnostics preview plus a screenshot under `test-results/native-windows/`;
7. stops the app and removes the test installation when uninstall metadata is available.

The harness deliberately does not click the native folder picker or read a real user project. Later project-operation suites must use disposable fixture directories.

## Native acceptance matrix

| Scenario                                  | CI automation |       Physical Win10/11 | Current evidence            |
| ----------------------------------------- | ------------: | ----------------------: | --------------------------- |
| Install unsigned NSIS artifact            |           Yes | Required before release | Windows Server 2022 CI only |
| Launch installed `Alicent.exe`            |           Yes | Required before release | Windows Server 2022 CI only |
| Prove Tauri/WebView2 boundary             |           Yes | Required before release | Diagnostics preview         |
| Project picker and filesystem permissions |            No |                Required | Pending                     |
| Keyboard/IME, scaling, screen reader      |            No |                Required | Pending                     |
| Uninstall and residue inspection          |   Best effort |                Required | Pending physical audit      |
| WCAG critical findings                    |  Must be zero |        Must be reviewed | axe report                  |

## Failure handling

Do not patch production UI, CSS, editor behavior, move/archive, or rich-text code from this test branch. Preserve the audit JSON and screenshot, then open a focused issue with:

- scope (`browser-smoke` or `installed-native-windows`);
- rule/check ID and severity;
- minimal reproduction;
- artifact name (never a local absolute path);
- explicit owner area so fixes can land in a non-overlapping branch.
