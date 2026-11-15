// SPDX-License-Identifier: MPL-2.0
#![allow(unsafe_op_in_unsafe_fn)]
use crate::{
    config::*,
    platform::{self, EventHooks, wide},
    runtime::{self, Command, Controller, Status},
    storage::Paths,
};
use std::{
    cell::RefCell,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::atomic::{AtomicIsize, Ordering},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::*, SystemServices::SS_LEFT, Threading::*},
    UI::{
        Accessibility::*, Controls::Dialogs::*, Controls::*, HiDpi::*, Input::KeyboardAndMouse::*,
        Shell::*, WindowsAndMessaging::*,
    },
};

const CLASS: &str = "FullscreenNightLight.Main.v1";
const EDITOR_CLASS: &str = "FullscreenNightLight.ApplicationEditor.v1";
const TRAY: u32 = WM_APP + 10;
const ACTIVATE: u32 = WM_APP + 11;
static MAIN: AtomicIsize = AtomicIsize::new(0);
const NAV: [&str; 5] = [
    "Dashboard",
    "Fullscreen Detection",
    "Applications",
    "Night Light",
    "General",
];
struct Control {
    hwnd: HWND,
    page: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}
struct Editor {
    hwnd: HWND,
    original: Option<AppRule>,
}
struct App {
    hwnd: HWND,
    controller: Controller,
    paths: Paths,
    config: Config,
    status: Status,
    page: usize,
    controls: Vec<Control>,
    font: HFONT,
    title_font: HFONT,
    dpi: u32,
    editor: Option<Editor>,
    initializing: bool,
    exiting: bool,
    stopped: bool,
    taskbar: u32,
    tray_ready: bool,
    last_tray: String,
    last_refresh: Instant,
    submitted: u64,
    scroll: POINT,
}

pub fn run() -> Result<(), String> {
    // SAFETY: All UI resources are owned and used on this thread. Every window's
    // user-data pointer remains valid until after its message loop has ended.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let name = wide(&format!(
            "Local\\FullscreenNightLight.{}",
            std::env::var("USERNAME").unwrap_or_else(|_| "User".into())
        ));
        let mutex = CreateMutexW(null(), 0, name.as_ptr());
        if mutex.is_null() {
            return Err(format!(
                "Create instance mutex: {}",
                std::io::Error::last_os_error()
            ));
        }
        let existing = GetLastError() == ERROR_ALREADY_EXISTS;
        let startup = std::env::args().any(|a| a == "--startup");
        let exit_requested = std::env::args().any(|a| a == "--exit");
        if existing {
            if !startup || exit_requested {
                for _ in 0..40 {
                    let hwnd = FindWindowW(wide(CLASS).as_ptr(), null());
                    if !hwnd.is_null() {
                        let mut process = 0;
                        GetWindowThreadProcessId(hwnd, &mut process);
                        AllowSetForegroundWindow(process);
                        PostMessageW(
                            hwnd,
                            if exit_requested { WM_COMMAND } else { ACTIVATE },
                            if exit_requested { 899 } else { 0 },
                            0,
                        );
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            CloseHandle(mutex);
            return Ok(());
        }
        if exit_requested {
            CloseHandle(mutex);
            return Ok(());
        }
        let result = run_instance(startup);
        CloseHandle(mutex);
        result
    }
}
unsafe fn run_instance(startup: bool) -> Result<(), String> {
    let paths = Paths::discover()?;
    let read_only = std::env::args().any(|a| a == "--read-only");
    let (controller, worker) = runtime::start(paths.clone(), read_only)?;
    let instance = GetModuleHandleW(null());
    let icc = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_STANDARD_CLASSES | ICC_LISTVIEW_CLASSES,
    };
    InitCommonControlsEx(&icc);
    for (name, proc) in [
        (
            CLASS,
            Some(window_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
        ),
        (
            EDITOR_CLASS,
            Some(editor_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
        ),
    ] {
        let class_name = wide(name);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: proc,
            hInstance: instance,
            hIcon: LoadIconW(instance, 1 as _),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            lpszClassName: class_name.as_ptr(),
            ..zeroed()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(format!(
                "Register native window: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    let hwnd = CreateWindowExW(
        WS_EX_CONTROLPARENT,
        wide(CLASS).as_ptr(),
        wide("Fullscreen Night Light").as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        1060,
        780,
        null_mut(),
        null_mut(),
        instance,
        null(),
    );
    if hwnd.is_null() {
        controller.send(Command::Exit)?;
        worker.join().map_err(|_| "Worker thread panicked")?;
        return Err(format!(
            "Create main window: {}",
            std::io::Error::last_os_error()
        ));
    }
    let status = controller.snapshot();
    let config = status.config.clone();
    let dpi = GetDpiForWindow(hwnd);
    let state = Box::new(RefCell::new(App {
        hwnd,
        controller: controller.clone(),
        paths,
        config,
        status,
        page: 0,
        controls: Vec::new(),
        font: null_mut(),
        title_font: null_mut(),
        dpi,
        editor: None,
        initializing: true,
        exiting: false,
        stopped: false,
        taskbar: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
        tray_ready: false,
        last_tray: String::new(),
        last_refresh: Instant::now(),
        submitted: 0,
        scroll: POINT { x: 0, y: 0 },
    }));
    SetWindowLongPtrW(
        hwnd,
        GWLP_USERDATA,
        (&*state as *const RefCell<App>) as isize,
    );
    let mut app = state.borrow_mut();
    MAIN.store(hwnd as isize, Ordering::Relaxed);
    controller.hwnd.store(hwnd as isize, Ordering::Relaxed);
    app.build();
    let file = CreatePopupMenu();
    AppendMenuW(file, MF_STRING, 800, wide("&Settings").as_ptr());
    AppendMenuW(file, MF_SEPARATOR, 0, null());
    AppendMenuW(file, MF_STRING, 899, wide("E&xit").as_ptr());
    let view = CreatePopupMenu();
    for (index, title) in NAV.iter().enumerate() {
        AppendMenuW(view, MF_STRING, 100 + index, wide(title).as_ptr());
    }
    let menu = CreateMenu();
    AppendMenuW(menu, MF_POPUP, file as usize, wide("&File").as_ptr());
    AppendMenuW(menu, MF_POPUP, view as usize, wide("&View").as_ptr());
    AppendMenuW(menu, MF_STRING, 606, wide("&License").as_ptr());
    SetMenu(hwnd, menu);
    DrawMenuBar(hwnd);
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(
        MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    ) != 0
    {
        let scale = |v: i32| v * dpi as i32 / 96;
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            scale(1060).min(monitor.rcWork.right - monitor.rcWork.left),
            scale(800).min(monitor.rcWork.bottom - monitor.rcWork.top),
            SWP_NOMOVE | SWP_NOZORDER,
        );
    }
    app.refresh();
    app.initializing = false;
    app.tray(true);
    let hooks = EventHooks::install(Some(window_event));
    if let Err(e) = &hooks {
        app.feedback(e);
    }
    if !startup && !app.config.start_minimized {
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
    }
    drop(app);
    let mut message: MSG = zeroed();
    loop {
        let result = GetMessageW(&mut message, null_mut(), 0, 0);
        if result <= 0 {
            if result < 0 {
                state.borrow().feedback("Windows message loop failed");
            }
            break;
        }
        let dialog = state
            .borrow()
            .editor
            .as_ref()
            .map(|e| e.hwnd)
            .unwrap_or(hwnd);
        if IsDialogMessageW(dialog, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    controller.hwnd.store(0, Ordering::Relaxed);
    MAIN.store(0, Ordering::Relaxed);
    drop(hooks);
    let app = state.borrow();
    if !app.stopped {
        controller.send(Command::Exit)?;
    }
    worker.join().map_err(|_| {
        "Background engine panicked; inspect recovery state on next launch".to_string()
    })?;
    Shell_NotifyIconW(NIM_DELETE, &app.tray_data());
    if !app.font.is_null() {
        DeleteObject(app.font);
    }
    if !app.title_font.is_null() {
        DeleteObject(app.title_font);
    }
    Ok(())
}
unsafe extern "system" fn window_event(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    object: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    if object == 0 {
        let hwnd = MAIN.load(Ordering::Relaxed) as HWND;
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_APP + 12, 0, 0);
        }
    }
}
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }
    if matches!(msg, WM_CTLCOLORSTATIC | WM_CTLCOLORBTN) {
        return control_background(w);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    // Default processing of nonclient mouse/system commands synchronously sends
    // WM_CLOSE (and other messages) back to this window. Do not hold a RefCell
    // borrow across DefWindowProcW: that would make the nested close bypass our
    // tray policy and destroy the window instead.
    let taskbar_message = (*pointer).try_borrow().is_ok_and(|app| msg == app.taskbar);
    if !taskbar_message
        && !matches!(
            msg,
            WM_COMMAND
                | runtime::STATUS_MESSAGE
                | runtime::STOPPED_MESSAGE
                | ACTIVATE
                | WM_NOTIFY
                | WM_APP12
                | WM_SIZE
                | DEFERRED_LAYOUT
                | WM_VSCROLL
                | WM_HSCROLL
                | WM_MOUSEWHEEL
                | WM_DPICHANGED
                | WM_SETTINGCHANGE
                | WM_THEMECHANGED
                | WM_GETMINMAXINFO
                | WM_CLOSE
                | WM_QUERYENDSESSION
                | WM_ENDSESSION
                | WM_POWERBROADCAST
                | WM_DISPLAYCHANGE
                | TRAY
        )
    {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    // Win32 synchronously reenters window procedures during control creation,
    // painting and resizing. Never alias a live mutable Rust reference.
    let Ok(mut app) = (*pointer).try_borrow_mut() else {
        if msg == WM_SIZE {
            PostMessageW(hwnd, WM_APP + 13, 0, 0);
        }
        if msg == WM_CLOSE {
            PostMessageW(hwnd, WM_CLOSE, w, l);
            return 0;
        }
        return DefWindowProcW(hwnd, msg, w, l);
    };
    if msg == app.taskbar {
        app.tray_ready = false;
        app.tray(true);
        return 0;
    }
    match msg {
        WM_COMMAND => {
            let id = (w & 0xffff) as u16;
            let notification = ((w >> 16) & 0xffff) as u16;
            if !app.initializing {
                app.command(id, notification);
            }
            0
        }
        runtime::STATUS_MESSAGE => {
            app.refresh();
            0
        }
        runtime::STOPPED_MESSAGE => {
            app.refresh();
            app.stopped = true;
            if app.exiting {
                if app.status.feedback.starts_with("Operation failed") {
                    MessageBoxW(
                        hwnd,
                        wide(&format!(
                            "{}\nRecovery information has been retained for the next launch.",
                            app.status.feedback
                        ))
                        .as_ptr(),
                        wide("Night Light restoration failed").as_ptr(),
                        MB_OK | MB_ICONWARNING,
                    );
                }
                DestroyWindow(hwnd);
            }
            0
        }
        ACTIVATE => {
            app.show();
            0
        }
        WM_NOTIFY => {
            let header = &*(l as *const NMHDR);
            if header.idFrom == 400
                && header.code == NM_DBLCLK
                && let Some(index) = app.selected_rule()
            {
                app.open_editor(Some(index));
            }
            0
        }
        WM_APP12 => {
            if app.last_refresh.elapsed().as_millis() >= 75 {
                app.last_refresh = Instant::now();
                app.send(Command::Refresh);
            }
            0
        }
        WM_SIZE | DEFERRED_LAYOUT => {
            app.layout();
            0
        }
        WM_VSCROLL | WM_HSCROLL => {
            let vertical = msg == WM_VSCROLL;
            let bar = if vertical { SB_VERT } else { SB_HORZ };
            let mut info = SCROLLINFO {
                cbSize: size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..zeroed()
            };
            GetScrollInfo(hwnd, bar, &mut info);
            let old = info.nPos;
            let position = match (w & 0xffff) as i32 {
                SB_LINEUP => old - 24,
                SB_LINEDOWN => old + 24,
                SB_PAGEUP => old - info.nPage as i32,
                SB_PAGEDOWN => old + info.nPage as i32,
                SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
                SB_TOP => 0,
                SB_BOTTOM => info.nMax,
                _ => old,
            };
            let position = position.clamp(0, (info.nMax - info.nPage as i32 + 1).max(0));
            if vertical {
                app.scroll.y = position;
            } else {
                app.scroll.x = position;
            }
            app.layout();
            0
        }
        WM_MOUSEWHEEL => {
            app.scroll.y = (app.scroll.y - ((w >> 16) as i16 as i32) / 120 * 48).max(0);
            app.layout();
            0
        }
        WM_DPICHANGED => {
            app.dpi = (w & 0xffff) as u32;
            let r = &*(l as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            app.fonts();
            app.layout();
            0
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            platform::dwm_dark(hwnd, app.dark_preference());
            InvalidateRect(hwnd, null(), 1);
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(l as *mut MINMAXINFO);
            let dpi = app.dpi as i32;
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            );
            info.ptMinTrackSize.x =
                (960 * dpi / 96).min(monitor.rcWork.right - monitor.rcWork.left);
            info.ptMinTrackSize.y =
                (700 * dpi / 96).min(monitor.rcWork.bottom - monitor.rcWork.top);
            0
        }
        WM_CLOSE => {
            if app.config.close_to_tray && !app.exiting {
                ShowWindow(hwnd, SW_HIDE);
            } else {
                app.exit();
            }
            0
        }
        WM_QUERYENDSESSION => {
            let (tx, rx) = std::sync::mpsc::channel();
            app.send(Command::PrepareSessionEnd(tx));
            match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => app.feedback(&format!(
                    "Shutdown restoration failed: {e}. Recovery retained."
                )),
                Err(e) => app.feedback(&format!(
                    "Shutdown restoration acknowledgement failed: {e}. Recovery retained."
                )),
            }
            1
        }
        WM_ENDSESSION => {
            if w != 0 {
                app.exit();
            } else {
                app.send(Command::Suspend(false));
            }
            0
        }
        WM_POWERBROADCAST => {
            if w as u32 == 4 {
                app.send(Command::Suspend(true));
            } else if matches!(w as u32, 7 | 18) {
                app.send(Command::Suspend(false));
            }
            1
        }
        WM_DISPLAYCHANGE => {
            app.send(Command::Refresh);
            0
        }
        TRAY => {
            match l as u32 {
                WM_LBUTTONDBLCLK => app.show(),
                WM_RBUTTONUP | WM_CONTEXTMENU => app.menu(),
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
const WM_APP12: u32 = WM_APP + 12;
const DEFERRED_LAYOUT: u32 = WM_APP + 13;
unsafe fn control_background(w: WPARAM) -> LRESULT {
    SetBkMode(w as HDC, TRANSPARENT as i32);
    SetTextColor(w as HDC, GetSysColor(COLOR_WINDOWTEXT));
    GetSysColorBrush(COLOR_WINDOW) as isize
}

impl App {
    #[allow(
        clippy::too_many_arguments,
        reason = "Native control geometry is kept explicit at call sites"
    )]
    unsafe fn add(
        &mut self,
        page: usize,
        class: &str,
        text: &str,
        id: u16,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        style: u32,
    ) -> HWND {
        let hwnd = CreateWindowExW(
            if class == "EDIT" || class == "LISTBOX" {
                WS_EX_CLIENTEDGE
            } else {
                0
            },
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            x,
            y,
            w,
            h,
            self.hwnd,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        self.controls.push(Control {
            hwnd,
            page,
            x,
            y,
            w,
            h,
        });
        hwnd
    }
    unsafe fn label(&mut self, page: usize, text: &str, id: u16, y: i32, h: i32) {
        self.add(page, "STATIC", text, id, 210, y, 740, h, SS_LEFT);
    }
    unsafe fn button(&mut self, page: usize, text: &str, id: u16, x: i32, y: i32, w: i32) {
        self.add(
            page,
            "BUTTON",
            text,
            id,
            x,
            y,
            w,
            32,
            WS_TABSTOP | BS_PUSHBUTTON as u32,
        );
    }
    unsafe fn check(&mut self, page: usize, text: &str, id: u16, y: i32) {
        self.add(
            page,
            "BUTTON",
            text,
            id,
            210,
            y,
            720,
            28,
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        );
    }
    unsafe fn combo(&mut self, page: usize, label: &str, id: u16, y: i32, values: &[&str]) {
        self.add(page, "STATIC", label, 0, 210, y, 310, 24, SS_LEFT);
        let hwnd = self.add(
            page,
            "COMBOBOX",
            "",
            id,
            530,
            y - 3,
            410,
            200,
            WS_TABSTOP | CBS_DROPDOWNLIST as u32 | WS_VSCROLL,
        );
        for value in values {
            SendMessageW(hwnd, CB_ADDSTRING, 0, wide(value).as_ptr() as isize);
        }
    }
    unsafe fn numeric(&mut self, label: &str, id: u16, y: i32) {
        self.add(1, "STATIC", label, 0, 210, y, 430, 24, SS_LEFT);
        let hwnd = self.add(
            1,
            "EDIT",
            "",
            id,
            670,
            y - 3,
            150,
            28,
            WS_TABSTOP | ES_NUMBER as u32 | ES_AUTOHSCROLL as u32,
        );
        SendMessageW(hwnd, EM_SETLIMITTEXT, 5, 0);
        self.add(1, "STATIC", "ms", 0, 830, y, 50, 24, SS_LEFT);
    }
    unsafe fn build(&mut self) {
        self.add(
            usize::MAX,
            "STATIC",
            "NIGHT LIGHT",
            900,
            22,
            22,
            170,
            32,
            SS_LEFT,
        );
        for (i, name) in NAV.iter().enumerate() {
            self.button(
                usize::MAX,
                name,
                100 + i as u16,
                16,
                86 + i as i32 * 48,
                175,
            );
        }
        self.add(usize::MAX, "STATIC", "", 901, 210, 22, 740, 40, SS_LEFT);
        self.add(
            usize::MAX,
            "EDIT",
            "",
            902,
            210,
            620,
            740,
            64,
            ES_MULTILINE as u32 | ES_READONLY as u32 | WS_VSCROLL | WS_TABSTOP,
        );
        self.label(0, "", 200, 85, 360);
        self.check(0, "Enable automatic Night Light management", 201, 460);
        self.button(0, "Open Settings", 202, 210, 510, 200);
        self.button(0, "Resume Management", 203, 430, 510, 230);
        self.combo(
            1,
            "Detection policy",
            300,
            85,
            &["Foreground Only", "Any Visible Fullscreen"],
        );
        self.label(1,"Foreground Only follows the active application on any monitor. Any Visible Fullscreen checks all visible windows using Z-order and occlusion.",320,125,48);
        self.combo(
            1,
            "Occlusion policy",
            301,
            185,
            &["Any Exposed Area", "Entire Display Exposed"],
        );
        self.label(1,"Exposed Area counts partially visible fullscreen apps. Entire Display ignores incidental overlays but stops for substantive overlap. Transparency is approximate.",321,225,48);
        self.check(
            1,
            "Include fullscreen ordinary applications and games",
            302,
            285,
        );
        self.check(1, "Include fullscreen browsers", 303, 320);
        self.check(1, "Include fullscreen video players", 304, 355);
        self.check(1, "Treat maximized windows as fullscreen", 305, 390);
        self.numeric("Detection confirmation delay", 310, 445);
        self.numeric("Exit confirmation delay", 311, 490);
        self.numeric("Polling fallback interval", 312, 535);
        self.label(1,"Categories use executable identity, not displayed content. Manage classifications on Applications.",322,580,30);
        self.label(2, "Application classifications and exclusions", 410, 85, 24);
        let list = self.add(
            2,
            "SysListView32",
            "",
            400,
            210,
            115,
            740,
            330,
            WS_TABSTOP | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
        );
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            0,
            (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP) as isize,
        );
        for (index, (title, width)) in [
            ("Executable name or exact path", 380),
            ("Category", 170),
            ("Excluded", 85),
            ("Match", 90),
        ]
        .iter()
        .enumerate()
        {
            let mut text = wide(title);
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM,
                cx: width * self.dpi as i32 / 96,
                pszText: text.as_mut_ptr(),
                iSubItem: index as i32,
                ..zeroed()
            };
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                index,
                (&column as *const LVCOLUMNW) as isize,
            );
        }
        self.button(2, "Add…", 401, 210, 465, 110);
        self.button(2, "Edit…", 402, 330, 465, 110);
        self.button(2, "Remove", 403, 450, 465, 110);
        self.button(2, "Toggle Exclusion", 404, 570, 465, 170);
        self.button(2, "Exclude Current App", 405, 750, 465, 200);
        self.label(2,"Exact-path classifications override name rules. Any matching exclusion takes priority. Select an entry to edit or remove it. Unrecognized apps are Ordinary.",411,520,65);
        self.label(3, "", 500, 85, 215);
        self.combo(
            3,
            "Manual override recovery",
            501,
            325,
            &["Next Fullscreen Session", "Explicit Resume Only"],
        );
        self.label(3,"Manual changes suspend automation. Next Session resumes only after fullscreen ends and a new session starts. Explicit Resume waits for your action.",510,375,60);
        self.button(3, "Resume Automatic Management", 502, 210, 460, 280);
        self.button(3, "Restore Night Light Now", 503, 510, 460, 260);
        self.button(3, "Open Windows Night Light Settings", 504, 210, 515, 400);
        self.label(3,"Scheduling and color temperature remain managed by Windows. Unavailable state is never guessed.",511,570,40);
        self.check(4, "Start with Windows (current user only)", 600, 85);
        self.check(4, "Start minimized to tray on manual launch", 601, 130);
        self.check(4, "Minimize to tray when closing the main window", 602, 175);
        self.check(4, "Enable diagnostic logging", 603, 220);
        self.check(
            4,
            "Disable Night Light during qualifying fullscreen sessions",
            607,
            265,
        );
        self.button(4, "Open Log Directory", 604, 210, 325, 240);
        self.button(4, "Open Configuration Directory", 605, 470, 325, 300);
        self.label(4,concat!("Fullscreen Night Light ", env!("CARGO_PKG_VERSION"), "\r\nWindows 11 x64 • Native Win32 interface\r\nMozilla Public License 2.0 (MPL-2.0)\r\nNo telemetry or network requirement."),610,395,130);
        self.button(4, "View MPL-2.0 License", 606, 210, 540, 260);
        self.fonts();
        self.sync_controls();
        self.navigate(0);
    }
    unsafe fn fonts(&mut self) {
        let old = self.font;
        let old_title = self.title_font;
        let px = |v: i32| v * self.dpi as i32 / 96;
        self.font = CreateFontW(
            -px(17),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        self.title_font = CreateFontW(
            -px(26),
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        for c in &self.controls {
            SendMessageW(
                c.hwnd,
                WM_SETFONT,
                if GetDlgCtrlID(c.hwnd) == 901 {
                    self.title_font
                } else {
                    self.font
                } as usize,
                1,
            );
        }
        if !old.is_null() {
            DeleteObject(old);
        }
        if !old_title.is_null() {
            DeleteObject(old_title);
        }
        platform::dwm_dark(self.hwnd, self.dark_preference());
    }
    unsafe fn dark_preference(&self) -> bool {
        use windows_sys::Win32::System::Registry::*;
        let mut value = 1u32;
        let mut size = 4;
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        );
        value == 0
    }
    unsafe fn layout(&mut self) {
        let scale = |v: i32| v * self.dpi as i32 / 96;
        let mut client: RECT = zeroed();
        GetClientRect(self.hwnd, &mut client);
        let width = client.right.max(scale(980));
        let height = client.bottom.max(scale(710));
        self.scroll.x = self.scroll.x.clamp(0, (width - client.right).max(0));
        self.scroll.y = self.scroll.y.clamp(0, (height - client.bottom).max(0));
        for (bar, max, page, pos) in [
            (SB_HORZ, width, client.right, self.scroll.x),
            (SB_VERT, height, client.bottom, self.scroll.y),
        ] {
            let info = SCROLLINFO {
                cbSize: size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: max - 1,
                nPage: page.max(0) as u32,
                nPos: pos,
                ..zeroed()
            };
            SetScrollInfo(self.hwnd, bar, &info, 1);
        }
        for c in &self.controls {
            let id = GetDlgCtrlID(c.hwnd);
            let y = if id == 902 {
                height - scale(82)
            } else {
                scale(c.y)
            };
            let width = if c.x == 210 && c.w == 740 {
                (width - scale(c.x + 24)).max(scale(c.w))
            } else {
                scale(c.w)
            };
            MoveWindow(
                c.hwnd,
                scale(c.x) - self.scroll.x,
                y - self.scroll.y,
                width,
                scale(c.h),
                1,
            );
        }
    }
    unsafe fn navigate(&mut self, page: usize) {
        self.page = page;
        self.scroll = POINT { x: 0, y: 0 };
        for c in &self.controls {
            ShowWindow(
                c.hwnd,
                if c.page == usize::MAX || c.page == page {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        SetWindowTextW(GetDlgItem(self.hwnd, 901), wide(NAV[page]).as_ptr());
        self.layout();
    }
    unsafe fn text(&self, id: i32, text: &str) {
        let hwnd = GetDlgItem(self.hwnd, id);
        if get_text(hwnd) != text {
            SetWindowTextW(hwnd, wide(text).as_ptr());
        }
    }
    unsafe fn checked(&self, id: i32) -> bool {
        SendMessageW(GetDlgItem(self.hwnd, id), BM_GETCHECK, 0, 0) == BST_CHECKED as isize
    }
    unsafe fn selection(&self, id: i32) -> usize {
        SendMessageW(GetDlgItem(self.hwnd, id), CB_GETCURSEL, 0, 0).max(0) as usize
    }
    unsafe fn sync_controls(&mut self) {
        self.initializing = true;
        for (id, value) in [
            (201, self.config.enabled),
            (302, self.config.include_ordinary),
            (303, self.config.include_browsers),
            (304, self.config.include_video),
            (305, self.config.include_maximized),
            (600, self.config.start_with_windows),
            (601, self.config.start_minimized),
            (602, self.config.close_to_tray),
            (603, self.config.diagnostic_logging),
            (607, self.config.disable_in_fullscreen),
        ] {
            SendMessageW(
                GetDlgItem(self.hwnd, id),
                BM_SETCHECK,
                if value { BST_CHECKED } else { BST_UNCHECKED } as usize,
                0,
            );
        }
        for (id, value) in [
            (300, self.config.detection as usize),
            (301, self.config.occlusion as usize),
            (501, self.config.recovery as usize),
        ] {
            SendMessageW(GetDlgItem(self.hwnd, id), CB_SETCURSEL, value, 0);
        }
        EnableWindow(
            GetDlgItem(self.hwnd, 301),
            (self.config.detection == DetectionPolicy::AnyVisibleFullscreen) as i32,
        );
        for (id, value) in [
            (310, self.config.entry_ms),
            (311, self.config.exit_ms),
            (312, self.config.poll_ms),
        ] {
            self.text(id, &value.to_string());
        }
        let list = GetDlgItem(self.hwnd, 400);
        let selected = SendMessageW(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
        SendMessageW(list, WM_SETREDRAW, 0, 0);
        SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
        for (row, r) in self.config.applications.iter().enumerate() {
            for (column, value) in [
                r.executable.as_str(),
                r.category.label(),
                if r.excluded { "Yes" } else { "No" },
                if r.exact_path {
                    "Exact path"
                } else {
                    "Filename"
                },
            ]
            .iter()
            .enumerate()
            {
                let mut text = wide(value);
                let item = LVITEMW {
                    mask: LVIF_TEXT,
                    iItem: row as i32,
                    iSubItem: column as i32,
                    pszText: text.as_mut_ptr(),
                    ..zeroed()
                };
                SendMessageW(
                    list,
                    if column == 0 {
                        LVM_INSERTITEMW
                    } else {
                        LVM_SETITEMTEXTW
                    },
                    if column == 0 { 0 } else { row },
                    (&item as *const LVITEMW) as isize,
                );
            }
        }
        if selected >= 0 {
            let item = LVITEMW {
                stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                state: LVIS_SELECTED | LVIS_FOCUSED,
                ..zeroed()
            };
            SendMessageW(
                list,
                LVM_SETITEMSTATE,
                selected as usize,
                (&item as *const LVITEMW) as isize,
            );
        }
        SendMessageW(list, WM_SETREDRAW, 1, 0);
        InvalidateRect(list, null(), 1);
        self.initializing = false;
    }
    unsafe fn refresh(&mut self) {
        let status = self.controller.snapshot();
        if status.config_ack >= self.submitted && self.config != status.config {
            self.config = status.config.clone();
            self.sync_controls();
        }
        self.status = status;
        let s = &self.status;
        let yn = |v| if v { "Yes" } else { "No" };
        let state = s
            .nightlight
            .map(|v| if v { "Enabled" } else { "Disabled" })
            .unwrap_or("Unavailable");
        let applications = if s.fullscreen.is_empty() {
            "None".into()
        } else {
            s.fullscreen
                .iter()
                .take(3)
                .map(|w| {
                    w.path
                        .as_deref()
                        .or(w.name.as_deref())
                        .unwrap_or("Executable unavailable")
                })
                .collect::<Vec<_>>()
                .join("\r\n")
        };
        let dashboard = format!(
            "Automatic management: {}\r\n\r\nNight Light: {state}\r\nFullscreen application detected: {}\r\nDetection policy: {}\r\nTemporary override owned: {}\r\nAutomation suspended: {}\r\n\r\nStatus: {}\r\n\r\nDetected applications:\r\n{applications}",
            if s.config.enabled {
                "Enabled"
            } else {
                "Disabled"
            },
            yn(!s.fullscreen.is_empty()),
            if s.config.detection == DetectionPolicy::ForegroundOnly {
                "Foreground Only"
            } else {
                "Any Visible Fullscreen"
            },
            yn(s.owned),
            yn(s.suspended),
            s.phase.label()
        );
        self.text(200, &dashboard);
        self.text(500,&format!("Night Light: {state}\r\n\r\nWindows schedule: {}\r\n\r\nTemporary override owned: {}\r\nAutomation suspended: {}\r\n\r\n{}",s.schedule,yn(s.owned),yn(s.suspended),s.phase.label()));
        self.text(
            902,
            &format!(
                "{}{}{}",
                s.warning.as_deref().unwrap_or(""),
                if s.warning.is_some() && !s.feedback.is_empty() {
                    "\r\n"
                } else {
                    ""
                },
                s.feedback
            ),
        );
        EnableWindow(GetDlgItem(self.hwnd, 203), s.suspended as i32);
        EnableWindow(GetDlgItem(self.hwnd, 502), s.suspended as i32);
        EnableWindow(GetDlgItem(self.hwnd, 503), s.owned as i32);
        EnableWindow(
            GetDlgItem(self.hwnd, 405),
            s.current
                .as_ref()
                .is_some_and(|w| w.path.is_some() || w.name.is_some()) as i32,
        );
        self.tray(false);
    }
    unsafe fn feedback(&self, text: &str) {
        self.text(902, text);
    }
    unsafe fn send(&self, command: Command) {
        if let Err(e) = self.controller.send(command) {
            self.feedback(&e);
        }
    }
    unsafe fn save(&mut self) {
        match self.config.validate() {
            Ok(()) => {
                self.feedback("Applying changes…");
                self.submitted += 1;
                self.send(Command::Configure(self.config.clone(), self.submitted));
            }
            Err(e) => {
                self.feedback(&e);
                self.config = self.status.config.clone();
                self.sync_controls();
            }
        }
    }
    unsafe fn command(&mut self, id: u16, notification: u16) {
        if (100..105).contains(&id) {
            self.navigate((id - 100) as usize);
            return;
        }
        if notification == EN_KILLFOCUS as u16 && (310..=312).contains(&id) {
            let value = get_text(GetDlgItem(self.hwnd, id as i32)).parse::<u64>();
            match value {
                Ok(v) => {
                    match id {
                        310 => self.config.entry_ms = v,
                        311 => self.config.exit_ms = v,
                        _ => self.config.poll_ms = v,
                    }
                    self.save();
                }
                Err(_) => {
                    self.feedback("Enter a whole number of milliseconds");
                    self.sync_controls();
                }
            }
            return;
        }
        if matches!(id, 300 | 301 | 501) && notification == CBN_SELCHANGE as u16 {
            match id {
                300 => {
                    self.config.detection = if self.selection(300) == 0 {
                        DetectionPolicy::ForegroundOnly
                    } else {
                        DetectionPolicy::AnyVisibleFullscreen
                    }
                }
                301 => {
                    self.config.occlusion = if self.selection(301) == 0 {
                        OcclusionPolicy::AnyExposedArea
                    } else {
                        OcclusionPolicy::EntireDisplayExposed
                    }
                }
                _ => {
                    self.config.recovery = if self.selection(501) == 0 {
                        RecoveryPolicy::NextFullscreenSession
                    } else {
                        RecoveryPolicy::ExplicitResumeOnly
                    }
                }
            }
            EnableWindow(
                GetDlgItem(self.hwnd, 301),
                (self.config.detection == DetectionPolicy::AnyVisibleFullscreen) as i32,
            );
            self.save();
            return;
        }
        if notification != BN_CLICKED as u16 && id != 400 {
            return;
        }
        match id {
            201 => {
                self.config.enabled = self.checked(201);
                self.save();
            }
            202 | 800 => {
                self.navigate(4);
                self.show();
            }
            203 | 502 | 803 => self.send(Command::Resume),
            503 | 804 => self.send(Command::Restore),
            302..=305 | 600..=603 | 607 => {
                let value = self.checked(id as i32);
                match id {
                    302 => self.config.include_ordinary = value,
                    303 => self.config.include_browsers = value,
                    304 => self.config.include_video = value,
                    305 => self.config.include_maximized = value,
                    600 => self.config.start_with_windows = value,
                    601 => self.config.start_minimized = value,
                    602 => self.config.close_to_tray = value,
                    603 => self.config.diagnostic_logging = value,
                    _ => self.config.disable_in_fullscreen = value,
                }
                self.save();
            }
            401 => self.open_editor(None),
            402 => {
                if let Some(i) = self.selected_rule() {
                    self.open_editor(Some(i));
                }
            }
            400 => {
                if notification == LBN_DBLCLK as u16
                    && let Some(i) = self.selected_rule()
                {
                    self.open_editor(Some(i));
                }
            }
            403 => {
                if let Some(i) = self.selected_rule() {
                    self.config.applications.remove(i);
                    self.sync_controls();
                    self.save();
                }
            }
            404 => {
                if let Some(i) = self.selected_rule() {
                    self.config.applications[i].excluded = !self.config.applications[i].excluded;
                    self.sync_controls();
                    self.save();
                }
            }
            405 | 805 => self.exclude_current(),
            504 => {
                if let Err(e) = platform::open("ms-settings:nightlight") {
                    self.feedback(&e);
                }
            }
            604 => {
                if let Err(e) = platform::open(&self.paths.logs.to_string_lossy()) {
                    self.feedback(&e);
                }
            }
            605 => {
                if let Err(e) = platform::open(&self.paths.config.to_string_lossy()) {
                    self.feedback(&e);
                }
            }
            606 => {
                let p = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.join("LICENSE")));
                if let Some(p) = p.filter(|p| p.exists()) {
                    if let Err(e) = platform::open(&p.to_string_lossy()) {
                        self.feedback(&e);
                    }
                } else {
                    self.feedback("MPL-2.0: license text is supplied in the portable distribution and repository LICENSE.");
                }
            }
            801 => self.show(),
            802 => {
                self.config.enabled = !self.config.enabled;
                self.sync_controls();
                self.save();
            }
            806 => {
                self.config.disable_in_fullscreen = !self.config.disable_in_fullscreen;
                self.sync_controls();
                self.save();
            }
            807 => {
                self.config.start_with_windows = !self.config.start_with_windows;
                self.sync_controls();
                self.save();
            }
            808 => {
                self.navigate(2);
                self.show();
            }
            899 => self.exit(),
            _ => {}
        }
    }
    unsafe fn selected_rule(&self) -> Option<usize> {
        let index = SendMessageW(
            GetDlgItem(self.hwnd, 400),
            LVM_GETNEXTITEM,
            usize::MAX,
            LVNI_SELECTED as isize,
        );
        if index >= 0 && (index as usize) < self.config.applications.len() {
            Some(index as usize)
        } else {
            self.feedback("Select an application entry first");
            None
        }
    }
    unsafe fn exclude_current(&mut self) {
        if let Some(w) = &self.status.current
            && let Some(identity) = w.path.as_ref().or(w.name.as_ref())
        {
            let exact = w.path.is_some();
            if let Some(rule) =
                self.config.applications.iter_mut().find(|r| {
                    r.exact_path == exact && normalize(&r.executable) == normalize(identity)
                })
            {
                rule.excluded = true;
            } else {
                self.config.applications.push(AppRule {
                    executable: identity.clone(),
                    exact_path: exact,
                    category: Category::Ordinary,
                    excluded: true,
                });
            }
            self.sync_controls();
            self.save();
        }
    }
    unsafe fn show(&self) {
        if let Some(editor) = &self.editor {
            ShowWindow(editor.hwnd, SW_RESTORE);
            SetForegroundWindow(editor.hwnd);
            return;
        }
        ShowWindow(self.hwnd, SW_RESTORE);
        // The first ShowWindow can inherit SW_HIDE from a startup launcher.
        // A second explicit call ensures subsequent manual activation is visible.
        ShowWindow(self.hwnd, SW_SHOW);
        SetForegroundWindow(self.hwnd);
    }
    unsafe fn exit(&mut self) {
        if self.stopped {
            DestroyWindow(self.hwnd);
            return;
        }
        if !self.exiting {
            self.exiting = true;
            self.feedback("Releasing owned Night Light override and exiting…");
            EnableWindow(self.hwnd, 0);
            self.send(Command::Exit);
        }
    }
    unsafe fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = TRAY;
        data.hIcon = LoadIconW(
            null_mut(),
            if self.status.warning.is_some() {
                IDI_ERROR
            } else if self.status.suspended
                || !self.config.enabled
                || self.status.phase == crate::engine::Phase::Paused
            {
                IDI_WARNING
            } else if self.status.owned {
                IDI_INFORMATION
            } else {
                IDI_APPLICATION
            },
        );
        if self.status.warning.is_none()
            && !self.status.suspended
            && self.config.enabled
            && !self.status.owned
            && self.status.phase != crate::engine::Phase::Paused
        {
            data.hIcon = LoadIconW(GetModuleHandleW(null()), 1 as _);
        }
        let text = format!(
            "Fullscreen Night Light — {}{}",
            self.status.phase.label(),
            if self.status.suspended {
                if self.config.recovery == RecoveryPolicy::NextFullscreenSession {
                    " (next fullscreen session)"
                } else {
                    " (explicit resume required)"
                }
            } else {
                ""
            }
        );
        for (a, b) in data.szTip.iter_mut().zip(text.encode_utf16().take(127)) {
            *a = b;
        }
        data
    }
    unsafe fn tray(&mut self, force: bool) {
        let key = format!(
            "{:?} {} {:?}",
            self.status.phase, self.status.owned, self.status.warning
        );
        if force || !self.tray_ready || key != self.last_tray {
            let data = self.tray_data();
            if Shell_NotifyIconW(if self.tray_ready { NIM_MODIFY } else { NIM_ADD }, &data) != 0 {
                self.tray_ready = true;
                self.last_tray = key;
            } else {
                self.feedback("System tray icon unavailable; use the main window to exit.");
            }
        }
    }
    unsafe fn menu(&self) {
        let menu = CreatePopupMenu();
        let status = wide(&format!("Status: {}", self.status.phase.label()));
        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, status.as_ptr());
        for (id, text, check) in [
            (801, "Open Application", None),
            (800, "Settings", None),
            (808, "Application Exclusions…", None),
            (802, "Enable Automation", Some(self.config.enabled)),
            (
                806,
                "Disable Night Light in Fullscreen",
                Some(self.config.disable_in_fullscreen),
            ),
            (
                807,
                "Start with Windows",
                Some(self.config.start_with_windows),
            ),
            (803, "Resume Automatic Management", None),
            (804, "Restore Night Light Now", None),
            (805, "Exclude Current Application", None),
            (899, "Exit", None),
        ] {
            AppendMenuW(
                menu,
                MF_STRING | if check == Some(true) { MF_CHECKED } else { 0 },
                id,
                wide(text).as_ptr(),
            );
        }
        let mut point = zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(self.hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            self.hwnd,
            null(),
        );
        if command != 0 {
            PostMessageW(self.hwnd, WM_COMMAND, command as usize, 0);
        }
        PostMessageW(self.hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }
    unsafe fn open_editor(&mut self, index: Option<usize>) {
        if let Some(editor) = &self.editor {
            SetForegroundWindow(editor.hwnd);
            return;
        }
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            wide(EDITOR_CLASS).as_ptr(),
            wide(if index.is_some() {
                "Edit Application"
            } else {
                "Add Application"
            })
            .as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            720,
            410,
            self.hwnd,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if hwnd.is_null() {
            self.feedback("Unable to create application editor");
            return;
        }
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            GetWindowLongPtrW(self.hwnd, GWLP_USERDATA),
        );
        let dpi = GetDpiForWindow(hwnd) as i32;
        let scale = |v| v * dpi / 96;
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            scale(720),
            scale(410),
            SWP_NOMOVE | SWP_NOZORDER,
        );
        let control = |class: &str, text: &str, id: i32, x, y, w, h, style| {
            let c = CreateWindowExW(
                if class == "EDIT" { WS_EX_CLIENTEDGE } else { 0 },
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                scale(x),
                scale(y),
                scale(w),
                scale(h),
                hwnd,
                id as usize as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            SendMessageW(c, WM_SETFONT, self.font as usize, 1);
            c
        };
        control(
            "STATIC",
            "Executable filename or exact path",
            0,
            24,
            20,
            620,
            24,
            0,
        );
        control(
            "EDIT",
            "",
            710,
            24,
            52,
            540,
            28,
            WS_TABSTOP | ES_AUTOHSCROLL as u32,
        );
        control("BUTTON", "Browse…", 714, 578, 50, 100, 32, WS_TABSTOP);
        control("STATIC", "Match by", 0, 24, 105, 180, 24, 0);
        let match_control = control(
            "COMBOBOX",
            "",
            711,
            220,
            102,
            458,
            160,
            WS_TABSTOP | CBS_DROPDOWNLIST as u32,
        );
        for text in ["Executable filename", "Exact executable path"] {
            SendMessageW(match_control, CB_ADDSTRING, 0, wide(text).as_ptr() as isize);
        }
        control("STATIC", "Application category", 0, 24, 160, 190, 24, 0);
        let category = control(
            "COMBOBOX",
            "",
            712,
            220,
            157,
            458,
            160,
            WS_TABSTOP | CBS_DROPDOWNLIST as u32,
        );
        for text in ["Ordinary Application", "Browser", "Video Player"] {
            SendMessageW(category, CB_ADDSTRING, 0, wide(text).as_ptr() as isize);
        }
        control(
            "BUTTON",
            "Exclude this application from automatic management",
            713,
            24,
            215,
            650,
            28,
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        );
        control("STATIC", "", 717, 24, 250, 650, 42, 0);
        control(
            "BUTTON",
            "Save",
            715,
            440,
            310,
            110,
            32,
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
        );
        control("BUTTON", "Cancel", 716, 568, 310, 110, 32, WS_TABSTOP);
        let rule = index
            .map(|i| self.config.applications[i].clone())
            .unwrap_or(AppRule {
                executable: String::new(),
                exact_path: false,
                category: Category::Ordinary,
                excluded: false,
            });
        SetWindowTextW(GetDlgItem(hwnd, 710), wide(&rule.executable).as_ptr());
        SendMessageW(match_control, CB_SETCURSEL, rule.exact_path as usize, 0);
        SendMessageW(category, CB_SETCURSEL, rule.category as usize, 0);
        SendMessageW(
            GetDlgItem(hwnd, 713),
            BM_SETCHECK,
            rule.excluded as usize,
            0,
        );
        self.editor = Some(Editor {
            hwnd,
            original: index.map(|i| self.config.applications[i].clone()),
        });
        EnableWindow(self.hwnd, 0);
        ShowWindow(hwnd, SW_SHOW);
        SetFocus(GetDlgItem(hwnd, 710));
    }
    unsafe fn close_editor(&mut self) {
        if let Some(editor) = self.editor.take() {
            EnableWindow(self.hwnd, 1);
            DestroyWindow(editor.hwnd);
            SetForegroundWindow(self.hwnd);
        }
    }
}
unsafe fn get_text(hwnd: HWND) -> String {
    let length = GetWindowTextLengthW(hwnd).max(0) as usize;
    let mut buffer = vec![0u16; length + 1];
    let n = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..n.max(0) as usize])
}
unsafe extern "system" fn editor_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if matches!(msg, WM_CTLCOLORSTATIC | WM_CTLCOLORBTN) {
        return control_background(w);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let Ok(mut app) = (*ptr).try_borrow_mut() else {
        return DefWindowProcW(hwnd, msg, w, l);
    };
    match msg {
        WM_COMMAND => {
            match (w & 0xffff) as u16 {
                714 => {
                    let mut file = vec![0u16; 32768];
                    let filter = wide("Windows executables (*.exe)\0*.exe\0\0");
                    let mut dialog: OPENFILENAMEW = zeroed();
                    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
                    dialog.hwndOwner = hwnd;
                    dialog.lpstrFilter = filter.as_ptr();
                    dialog.lpstrFile = file.as_mut_ptr();
                    dialog.nMaxFile = file.len() as u32;
                    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
                    if GetOpenFileNameW(&mut dialog) != 0 {
                        SetWindowTextW(GetDlgItem(hwnd, 710), file.as_ptr());
                        SendMessageW(GetDlgItem(hwnd, 711), CB_SETCURSEL, 1, 0);
                    } else {
                        let error = CommDlgExtendedError();
                        if error != 0 {
                            SetWindowTextW(
                                GetDlgItem(hwnd, 717),
                                wide(&format!("File picker failed: {error}")).as_ptr(),
                            );
                        }
                    }
                }
                715 | 1 => {
                    let rule = AppRule {
                        executable: get_text(GetDlgItem(hwnd, 710)).trim().into(),
                        exact_path: SendMessageW(GetDlgItem(hwnd, 711), CB_GETCURSEL, 0, 0) == 1,
                        category: match SendMessageW(GetDlgItem(hwnd, 712), CB_GETCURSEL, 0, 0) {
                            1 => Category::Browser,
                            2 => Category::VideoPlayer,
                            _ => Category::Ordinary,
                        },
                        excluded: SendMessageW(GetDlgItem(hwnd, 713), BM_GETCHECK, 0, 0)
                            == BST_CHECKED as isize,
                    };
                    let mut candidate = app.config.clone();
                    if let Some(original) = app.editor.as_ref().and_then(|e| e.original.as_ref()) {
                        let Some(i) = candidate.applications.iter().position(|r| r == original)
                        else {
                            SetWindowTextW(
                                GetDlgItem(hwnd, 717),
                                wide("This entry changed while editing. Cancel and reopen it.")
                                    .as_ptr(),
                            );
                            return 0;
                        };
                        candidate.applications[i] = rule;
                    } else {
                        candidate.applications.push(rule);
                    }
                    match candidate.validate() {
                        Ok(()) => {
                            app.config = candidate;
                            app.sync_controls();
                            app.save();
                            app.close_editor();
                        }
                        Err(e) => {
                            SetWindowTextW(GetDlgItem(hwnd, 717), wide(&e).as_ptr());
                        }
                    }
                }
                716 | 2 => app.close_editor(),
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            app.close_editor();
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
