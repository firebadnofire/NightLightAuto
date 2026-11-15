# Validation record

## Close-to-tray regression

Computer Use reproduced the installed app terminating when the actual title-bar X
was clicked, despite the checked close-to-tray option. The earlier close-to-tray
acceptance claim missed this path and was insufficient.

Default Win32 processing of nonclient/system commands synchronously sends WM_CLOSE.
The GUI previously held a mutable RefCell borrow during that default processing,
so the nested close fell through to DefWindowProc and destroyed the window. Default
messages now bypass the borrow; a busy close is deferred rather than destroying it.

The rebuilt release was tested with Computer Use and an isolated read-only profile.
The actual X and Alt+F4 both hid the window while preserving the same live PID;
reopening restored that instance. The 37 Rust tests, formatting and strict Clippy
passed, and the installer and portable package were rebuilt. No real Night Light
or startup preference was changed by the read-only regression tests. The installed
copy requires updating using the rebuilt installer.

## Release CI additions

- Forgejo workflow YAML parses and selects `ubuntu-latest` for both `v*` and `V*`.
- An isolated tagged `V0.2.7` Git fixture completed the Ubuntu/WSL MSVC cross-build
  and NSIS installer compilation, portable ZIP, dependency notices and source ZIP.
  Windows reads both FileVersion and ProductVersion as `0.2.7`. Source archive
  Cargo.toml/Cargo.lock contain that same derived version. The main checkout's
  Cargo version and Git tags were not changed by this fixture.
- 7-Zip recognized the generated setup as NSIS 3 Unicode and passed its complete
  archive integrity test, including the GUI payload, documentation and uninstaller.
- All five actual fixture release files, including SHA256SUMS, were signed and
  verified using a disposable GPG key. No production secrets were used.
- 14 release-tool regression tests pass: native-path NSIS compilation with spaces,
  version validation and tag/SHA consistency,
  repeatable version derivation, lowercase dependency licenses, unreviewed-license
  rejection, real GPG signing/checksums/tampering, credential host restrictions,
  non-forced tag push, draft uploads and public-asset overwrite prevention.
- Existing 37 Rust tests and strict Clippy still pass. Windows builds with
  `-NoInstaller` and `-NoPortable` work, including PowerShell 5.1.

The cross-linker reported LNK4099 warnings because Microsoft's static libraries
refer to PDB files absent from the downloaded SDK. Compilation succeeded; these
debug-symbol warnings were not hidden. The native Windows makensis invocation
was subsequently verified using the default
PowerShell 5.1 build from `build-scripts/windows`. A mixed-separator file lookup
failure was fixed by consistently using NSIS backslashes in the shared template;
both native Windows and Ubuntu compilers now succeed. The native setup also passes
7-Zip archive verification. Installer install/upgrade/uninstall remain untested.
Actual Forgejo execution, production-key signing and both release API
uploads await a real release tag; no public release or tag was pushed during tests.

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
