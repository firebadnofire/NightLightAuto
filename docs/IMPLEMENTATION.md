# Implementation contract

Implement the approved native Win32 GUI plan in units: project foundation; pure
configuration/detection/state logic; Windows integration; five-page graphical
application and tray; build/cleanup, notices, and documentation. Validate each
unit before moving on. No automated test may change real Night Light state.

Required GUI: Dashboard, Fullscreen Detection, Applications, Night Light, General.
All user settings must be accessible without editing TOML. Manual launch opens
the window by default; --startup hides it. Closing hides by default; Exit releases
owned overrides. A second manual launch focuses the existing window.

Both detection, occlusion, and manual-recovery policies are required. Exclusions
precede path categories, which precede name categories. Default timing is
500/750/500 ms. All categories enabled; maximize option off. Scheduling and color
temperature must never be written. Ambiguous external changes yield control.

Windows 11 x64 only, Rust 2024, MPL-2.0, no elevation, no network at runtime.
Build and cleanup scripts belong only under build-scripts/windows. Package the
complete graphical executable, documentation, license, and dependency notices.

## Section 5 — Mandatory graphical desktop interface

Use native Win32 windows, common controls, dialogs and menus with keyboard
navigation. No webview, Electron, Tauri, terminal settings interface or tray-only
implementation is acceptable. Keep the Rust engine independent and unit-testable.

1. Dashboard: live automation, Night Light, fullscreen detection, application and
   executable, policy, override, suspension and backend/recovery status; working
   enable/disable, settings and resume controls.
2. Fullscreen Detection: both detection and occlusion policies, three category
   inclusion flags, maximized-window handling, entry/exit delays and polling.
   Explain policies and disable controls when inapplicable.
3. Applications: classified executable table with category/exclusion, Add/Edit/Remove,
   exclusion toggle, executable file picker and exclude-current action. Changes apply
   immediately and persist across restarts.
4. Night Light: actual status and available schedule, both manual-recovery policies,
   suspension, resume and restore controls, errors, and Windows Settings shortcut.
   Display unavailable data honestly; do not reproduce Windows scheduling/temperature.
5. General: startup, start minimized, close to tray, diagnostic logging, open log/config
   directories, version and license.

Manual launch opens the window by default; --startup is silent. Closing hides by
default. Tray supplies Open Application, Settings and Exit, and double-click focuses
the window. A second launch focuses the existing instance. Explicit Exit releases
owned overrides. Support DPI scaling, practical Windows appearance integration,
accessible labels, clear feedback and nonblocking background work without elevation.

### Final GUI acceptance gate

Verify the main window and all five pages, every advertised control, immediate engine
updates and persisted settings, live Dashboard state, complete graphical application
management, both tray/window directions, manual/silent startup and single-instance
activation, complete portable runtime packaging, and no feature requiring TOML edits.
Record unperformed checks explicitly; do not declare full acceptance until they pass.
Retain MPL-2.0 and build-scripts/windows/build.ps1 and clean.ps1.
