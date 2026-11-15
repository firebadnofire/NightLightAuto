# Validation record

Validated on Windows 11 x64 build 26200 with Rust 1.97.1, MSVC 14.44,
Windows SDK and both Windows PowerShell 5.1 and PowerShell 7.

## Automated checks

- 37 unit tests pass. Coverage includes classification and exclusion precedence,
  visibility and occlusion, debounce, schedule boundaries and midnight, manual
  intervention, both recovery policies, shutdown and crash ownership, persistence,
  invalid configuration preservation, and bounded log rotation.
- `cargo fmt --check` and strict Clippy pass.
- Windows release/debug build and packaging scripts run successfully. Cleanup
  fixtures exercise dry-run, repeated cleanup, preservation of unknown/modified
  files, and rejection of paths outside the build directories.
- The release executable is an x64 Windows GUI executable with static CRT and
  Windows system DLL imports; no Qt or separately installed GUI runtime is needed.
  The ZIP contains the executable, README, MPL-2.0 license, example configuration,
  manual checklist, and generated third-party license notices.

## Interactive checks

GUI checks used `--read-only` and isolated APPDATA/LOCALAPPDATA directories.
They did not change the desktop's Night Light state or Windows startup registration.

- All five navigation pages rendered. The Dashboard displayed real backend status
  and available scheduling information.
- Native application editor opened and saved a test classification. Table exclusion,
  category settings, detection policy and numeric delay changes persisted.
- Restart preserved configuration. A packaged executable launched from a directory
  containing spaces.
- Closing hid the window while keeping the process alive; reopening used the existing
  instance. `--startup` hid the window; manual reopening displayed it. Explicit Exit
  terminated the process cleanly.

## Outstanding acceptance

Implementation is available, but full production acceptance is not claimed. Follow
[MANUAL-TESTS.md](MANUAL-TESTS.md) before release certification. Outstanding checks
include live Night Light disable/restore and manual intervention on real Windows,
actual startup registration, every control and file-picker interaction, application
edit/remove flows, prolonged live fullscreen Dashboard transitions, tray double-click,
exclusive games, multiple physical monitors and mixed DPI, accessibility/high contrast,
Explorer restart, real sleep/logout/shutdown and schedule transitions, and launch on
a clean Windows 11 installation. Unit-test simulations do not replace those checks.

The Night Light backend uses undocumented Windows data and can require updates after
Windows changes. Unavailable or ambiguous backend data is surfaced rather than invented.
