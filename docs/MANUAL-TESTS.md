# Manual Windows acceptance checklist

Use an interactive Windows 11 x64 desktop. Record OS build, display topology,
scaling, GPU/driver and tested application versions. Snapshot Night Light state,
schedule and temperature before any live write test. Do not automate changes on
a user's active desktop without their explicit control. Restore preferences at
the end. `--read-only` is suitable for GUI inspection without Night Light writes.

## Graphical application

- Launch manually: Dashboard opens, with no console. Visit all five pages.
- Resize and use 100%, 125%, 150%, 200% scaling; move between mixed-DPI monitors.
  Check text, scrolling, table columns, editor, file picker, and high contrast.
- Tab/Shift+Tab, arrows, Space, Enter and Escape navigate and operate controls.
- Change each setting; verify inline feedback and persisted value after restart.
  Invalid numeric values and duplicate rules show errors without applying them.
- Check live Dashboard state as foreground/fullscreen apps change.
- Add/edit/remove Ordinary/Browser/Video rules by filename and file picker path.
  Toggle exclusion, exclude current external app, verify exact-path precedence.
- Confirm occlusion selector disables in foreground mode and enables in any-visible.
- Verify both recovery options and their descriptions, directory shortcuts,
  license display, and Windows Night Light Settings shortcut.
- Close hides by default; tray Open/Settings/double-click restore/focus. A second
  launch focuses the first process. `--startup` shows no window. Exit terminates.
  Click the actual title-bar X and also test Alt+F4, rather than only sending
  WM_CLOSE directly. After each close, confirm the same process/PID remains alive
  and reopening restores that instance; a disappearing window alone is insufficient.
- Disable close-to-tray: close exits. Enable start-minimized: manual launch hides.
- Restart Explorer and confirm tray recreation. Confirm all tray/menu actions.
- With explicit authorization, toggle startup; inspect exactly one quoted HKCU Run
  value, including a path with spaces. Disable and verify its removal.

## Detection and actual Night Light control — requires deliberate live testing

- Enabled Night Light: fullscreen disables; exit restores. Already disabled:
  entry/exit before a boundary makes no writes. Check schedule/temperature unchanged.
- Test a borderless game, exclusive fullscreen where observable, browser F11,
  video fullscreen, ordinary/maximized windows, and auto-hidden taskbar.
- Test both detection policies on secondary/mixed-DPI monitors. Partial and full
  opaque overlap, minimized/hidden/cloaked apps, transparent/tool/notification
  overlays, disconnected/reconnected monitors, exclusions and policy changes.
- Alt+Tab shorter/longer than exit delay; consecutive fullscreen applications.
- Schedule activation/deactivation during fullscreen, including overnight manual
  schedules and Windows solar scheduling. Entry off before sunset exits on after
  sunset; entry on before sunrise exits off after sunrise.
- Manual changes while managed: no write battle. Default resumes next session;
  explicit policy remains suspended until Resume, including after restart.
- Change recovery policy while suspended; do not resume current session implicitly.
- Restore Now releases and suspends; explicit Resume can manage again.
- Disable automation/feature while owned; pause, normal Exit, logout/shutdown,
  suspend/resume, and Explorer restart. Confirm restoration or an accurate warning.
- Terminate the process during ownership. No immediate cleanup is expected.
  Restart: matching valid record recovers; newer external state does not get
  overwritten. An interrupted/ambiguous record warns and skips restoration.
- Change clock/timezone around a boundary; confirm conservative backwards-clock
  behavior. Unavailable solar/state data must display unavailable, never a guess.
- Simulate backend/read/write/persistence failures and confirm visible errors,
  bounded retry, responsive GUI, and preserved recovery evidence.

## Packaging and resource use

- Run build.ps1 in Windows PowerShell 5.1 and PowerShell 7, release and debug,
  from outside the repository. Run `-Test -Check`; verify meaningful failures.
- Extract ZIP on a clean Windows 11 machine without Rust, VS, Qt or added DLLs.
  Open every page and verify state, settings persistence, tray and launch modes.
- Inspect PE subsystem, manifest and DLL imports; no console or elevation prompt.
- Observe idle CPU/memory for at least 10 minutes and during any-visible polling.
  Check no busy loop, unbounded memory, registry-write storm, or network traffic.
- Preview cleanup, execute twice, preserve a deliberately unrecorded sentinel,
  a modified output, sources/lockfile and AppData. Reject a ledger path outside
  project roots and a symlink/junction. Rebuild successfully afterward.
