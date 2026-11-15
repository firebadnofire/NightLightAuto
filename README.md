# Fullscreen Night Light

A native Rust/Win32 Windows 11 x64 desktop utility that temporarily disables
Windows Night Light for qualifying fullscreen applications. It includes a live
Dashboard, detection settings, application classification/exclusion management,
Night Light status and recovery controls, General preferences, and a system tray.
No administrator privileges, webview, Qt installation, telemetry, or runtime
network access is required.

## Run and use

Extract the entire portable ZIP and launch `FullscreenNightLight.exe`. Manual
launch opens the main window by default. Close hides it in the tray by default;
double-click the tray icon or select **Open Application** to reopen it. Launching
another copy focuses the existing window. **Exit** releases an owned override
and terminates. The File menu also provides Exit without visiting the tray.

All options are graphical; editing TOML is never required:

- **Dashboard:** live automation, Night Light, fullscreen apps, policy, ownership,
  suspension, and warnings. Enable/disable automation or resume management.
- **Fullscreen Detection:** select detection/occlusion policies, category
  inclusion, maximized-window behavior, and timing. Settings apply immediately,
  subject to the configured entry/exit debounce.
- **Applications:** add/edit/remove filename or exact-path rules, browse for an
  executable, classify it as Ordinary, Browser, or Video Player, and toggle its
  exclusion. Exclude Current App uses the last foreground app outside this utility.
- **Night Light:** see available Windows state/schedule, choose manual recovery,
  resume automation, release an owned override, or open Windows Night Light
  Settings. Windows retains responsibility for schedule and temperature editing.
- **General:** startup, launch/close behavior, logging, directory shortcuts, and
  version/license. Startup is disabled by default.

**Restore Night Light Now** releases only a managed override and suspends
reapplication for that fullscreen session; it does not blindly turn Night Light
on. Resume is an explicit request to manage the current session again.

## Detection

**Foreground Only** (default) checks the active application on whichever monitor
it occupies. **Any Visible Fullscreen** checks top-level windows in Z-order on
connected monitors. Both use the same physical-pixel/DWM-bound classifier, with
a two-pixel edge tolerance. Ordinary framed/maximized windows are rejected,
including maximized windows beside an auto-hidden taskbar. The optional maximized
setting admits genuinely maximized application windows explicitly.

Hidden, minimized, cloaked, shell/desktop, utility, and incidental tool windows
do not qualify. Multi-monitor coordinates may be negative. Per-monitor DPI
awareness avoids mixing logical window sizes with physical display bounds.

In any-visible mode, **Any Exposed Area** counts an exposed rectangle of at least
8 by 8 physical pixels. **Entire Display Exposed** rejects substantive overlap
of that size. Opaque higher windows are subtracted; tooltips/tool windows and
transparent, layered, click-through, or recognized shell overlays do not count
as opaque blockers. Exact compositor visibility is not available from metadata:
rounded corners, complex regions, layered apps, notifications, capture overlays,
and unusual exclusive-fullscreen games can produce imperfect results. An
excluded window cannot trigger management but can still obscure another window.

Window events trigger coalesced reevaluation, with fallback polling. Defaults:
entry 500 ms, exit 750 ms, polling 500 ms. Brief Alt+Tab losses within the exit
delay remain one session. Foreground mode avoids enumerating all windows;
any-visible mode performs broader enumeration and can cost more CPU.

Categories are based on executable identities, never titles or displayed content.
Unrecognized apps are Ordinary. Seeded browsers include Firefox, Chrome, Edge,
Brave, Vivaldi, Opera, Waterfox and LibreWolf; players include VLC, mpv, MPC-HC,
MPC-BE and PotPlayer. Any matching exclusion takes precedence, then exact-path
classification, then filename classification. Names/paths match without case
sensitivity. An inaccessible executable is displayed as unavailable and treated
as Ordinary; identity-based rules cannot match an identity Windows denies.

## Night Light and schedule preservation

The application directly uses `win-nightlight-lib` and `win-nightlight-watch`
0.5.0; it never invokes an external `wnl.exe`. Only the active-state record is
written. Scheduling, manual times, solar mode, color temperature, and preview
settings are never written by this application.

On confirmed fullscreen entry it captures current state and schedule, writes a
recovery intent before disabling an enabled Night Light, and verifies readback.
If already off it makes no unnecessary write. It tracks scheduled changes and
keeps Night Light off while ownership remains valid. On exit, it restores the
captured manual state unless a known schedule boundary occurred; then it uses
current scheduled intent. Manual schedules spanning midnight are supported.
Windows-provided sunset/sunrise times are used without location/network queries.
A known boundary can also be inferred when Windows emits no active-state change.

Application echoes are matched against exact readback fingerprints. Scheduled
transitions are distinguished using the library's transition cause. An unexplained
external write, explicit schedule change, preview, or backwards clock change yields control
instead of fighting the user. Changing only color temperature does not require
rewriting or restoring settings.

Routine Windows refreshes of sunset/sunrise times update the captured schedule without
being treated as manual intervention.

Manual recovery policies:

- **Next Fullscreen Session** (default): suspend this session; resume after its
  debounced end and a subsequent entry, or use Resume explicitly.
- **Explicit Resume Only:** keep suspension across sessions and restarts until
  Resume. Changing policies during a suspended session does not resume it.

### Reverse-engineered API limitations

The libraries rely on **undocumented Windows implementation details** and may
need updates after Windows changes. A decoded registry state is not proof of
the actual displayed color transform, especially with HDR, remote desktops,
unsupported displays, exclusive fullscreen, or display-driver behavior.

Windows reports a "manual" transition for both people and programs; the watcher
can coalesce notifications. There is no atomic compare-and-swap or authoritative
writer identity. The app checks immediately before writing but cannot eliminate
every concurrent race. Unobservable actions that produce no state change cannot
be detected. Solar times have no reliable per-day validity proof; missing/equal
times are treated as unavailable, and no location-based guesses are made. Civil
clock changes moving backwards yield conservatively; forward changes account
for crossed known boundaries. Schedule changes during ownership suspend rather
than silently inventing a new restoration state.

Normal Exit/pause/suspend releases owned overrides. Windows session-ending
notifications allow a bounded restoration attempt; shutdown deadlines can still
interrupt it. Forced termination cannot perform immediate cleanup. On next
launch, a versioned recovery record no older than seven days is used only when
the exact owned state and schedule still match. Ambiguous/interrupted-write or
newer-user-change cases warn and skip restoration. Review Windows Settings when
such a warning appears. Session-only suspension is not persisted as permanent;
explicit suspension is.

## Configuration, startup, and logs

- Configuration: `%APPDATA%\FullscreenNightLight\config.toml`
- Logs: `%LOCALAPPDATA%\FullscreenNightLight\logs\`
- Recovery/suspension: `%LOCALAPPDATA%\FullscreenNightLight\`

Writes use flushed same-directory temporary files and atomic replacement.
Malformed configurations produce visible warnings and safe defaults. Invalid
originals are preserved before a graphical save replaces them. Valid external
edits reload, though startup registration changes require the GUI. Failed saves
retain the previous engine configuration and report failure.

Logs rotate at approximately 1 MiB each, keeping five files. Errors are logged;
optional diagnostics add state transitions. Window titles and unrelated activity
are not logged. All data stays local.

Start with Windows manages only the `FullscreenNightLight` value under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, with a quoted executable
path and `--startup`. Startup launch is always silent. Move the executable only
after disabling startup; enable it again from the new location. Windows may
independently disable startup applications in Task Manager.

The title bar follows Windows dark preference where supported. Native common
controls retain their accessible system appearance; full custom dark rendering
is intentionally not used. High contrast and native keyboard navigation are
retained. The main window supports DPI changes and scrolling on smaller displays.

## Build, test, and clean

Install Rust through rustup and Visual Studio 2022/Build Tools with **Desktop
development with C++**, MSVC x64 tools, and a Windows SDK. Rust 1.95+ is required
by the Night Light crates; `rust-toolchain.toml` pins the validated 1.97.1 toolchain.
If needed, install it with:

```powershell
rustup toolchain install 1.97.1 --component rustfmt --component clippy --target x86_64-pc-windows-msvc
```

Install [NSIS 3](https://nsis.sourceforge.io/) for the default installer output.
The script finds `makensis.exe` on PATH or in `%ProgramFiles(x86)%\NSIS`.

Normal workflow (Windows PowerShell 5.1 or PowerShell 7, from any directory):

```powershell
.\build-scripts\windows\build.ps1
.\build-scripts\windows\build.ps1 -Debug
.\build-scripts\windows\build.ps1 -Test -Check
.\build-scripts\windows\build.ps1 -NoInstaller
.\build-scripts\windows\build.ps1 -NoPortable
.\dist\windows\FullscreenNightLight\FullscreenNightLight.exe
```

The script builds with locked dependencies, static CRT linkage and embedded GUI
resources, checks dependency licenses, writes complete dependency notices, and
produces `dist\windows\FullscreenNightLight-portable.zip` and
`dist\windows\FullscreenNightLight-<version>-windows-x64-setup.exe` by default.
Use `-NoInstaller` and/or `-NoPortable` to skip outputs. Both packaging paths share
the same native application payload. No Qt or custom DLL is
required; standard Windows system DLLs remain prerequisites. Debug and release
builds share distribution paths, so the last successful build determines contents.

Forgejo CI cross-builds releases on `ubuntu-latest` for `v*` and `V*` tags,
derives the application version from the tag, signs all release payloads and
SHA256SUMS with OpenPGP, and mirrors releases to GitHub. The local and CI installer
use the same NSIS definition. See [release setup and signing](docs/RELEASING.md).

```powershell
.\build-scripts\windows\clean.ps1 -WhatIf
.\build-scripts\windows\clean.ps1
```

Cleanup uses a generated-file ownership ledger. It removes unchanged recorded
Cargo outputs, caches, ZIPs and distribution files; preserves unknown/modified
files; rejects paths outside output roots and junction/symlink traversal; and
removes only empty directories. It never touches global Cargo caches, sources,
the lockfile, Git metadata, or user AppData. A missing ledger results in no
deletion. Keep the ledger when moving build outputs. Cleanup is repeatable.

`--read-only` is an inspection mode that blocks Night Light and startup writes
while allowing GUI/configuration testing. Ordinary unit tests use simulated
backends and do not touch real Night Light. See [manual acceptance checklist](docs/MANUAL-TESTS.md)
and [validation results](docs/VALIDATION.md) for tested and outstanding scenarios.

## Troubleshooting

- **Unavailable/backend error:** open Windows Night Light Settings, check display
  driver support and Windows updates. Decode errors may require an updated library.
  The app displays errors and retries with bounded backoff instead of write loops.
- **Unexpected detection:** check policy, exclusions, categories, maximized option,
  occlusion limitations, and the executable shown on Dashboard.
- **Manual change suspended management:** use Resume or wait for the next session
  under the default recovery policy. Restore Now intentionally suspends reapplication.
- **Configuration error:** use the GUI to save corrected settings; inspect the
  preserved original in the configuration directory if needed.
- **Tray icon missing:** open the executable again to focus its window. Explorer
  restarts trigger icon recreation; Exit remains available in the File menu.
- **Build failure:** check the reported toolchain/SDK error. Run with `-Test -Check`;
  do not bypass failed checks or assume a previous ZIP contains the new build.

## License and compatibility

Application source is licensed under **MPL-2.0**, with the complete license in
[LICENSE](LICENSE) and SPDX headers in source files. Dependencies retain their
own licenses. The locked dependency set uses permissive MIT/Apache-2.0,
Unicode-3.0 and Unlicense alternatives compatible with distribution alongside
MPL-2.0 source. Build-time and runtime license texts are included in the portable
package's `THIRD-PARTY-NOTICES.txt`; unknown license expressions fail packaging.
MPL source-availability obligations apply when distributing modified covered
source; preserve notices and provide the corresponding source to recipients.

Target: Windows 11 x64 only. Development/GUI inspection was performed on Windows
11 build 26200; this is not a claim of compatibility with untested Windows builds
or complete game/display-hardware certification. No Linux/macOS implementation.
