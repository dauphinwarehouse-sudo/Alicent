# Diagnostics preview contract

`tests/contracts/diagnostics-preview.schema.json` defines `alicent.diagnostics-preview/v1`. Both browser and installed-native audits emit this shape so CI and release review can compare results without confusing their evidence.

## Required guarantees

- `scope.mode` is either `browser-smoke` or `installed-native-windows`.
- Browser smoke always reports `installedApplication: false` and `nativeBoundaryExercised: false`.
- Installed-native results report whether the Tauri boundary was actually observed; the harness fails if it was not.
- The environment contains only bounded technical metadata. The contract excludes project text, credentials, full executable paths, usernames, and home directories.
- Artifact paths are relative to `test-results/`; absolute paths and parent traversal are rejected by the contract test.
- Accessibility counts and rule summaries are machine-readable. The CI gate requires zero `critical` axe findings. Lower severities remain visible in the artifact and are triaged as separate issues rather than fixed in this branch.

## Artifacts

| Producer                 | Preview                                                | Companion evidence  |
| ------------------------ | ------------------------------------------------------ | ------------------- |
| Browser smoke            | `test-results/accessibility/browser-smoke.json`        | `browser-smoke.png` |
| Installed native Windows | `test-results/native-windows/diagnostics-preview.json` | `installed-app.png` |

The preview is diagnostic evidence, not a claim that every P5.2 scenario or release criterion passed.
