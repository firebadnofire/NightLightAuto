// SPDX-License-Identifier: MPL-2.0
use crate::{
    detection::{Rect, Window, WindowSource},
    engine::{Clock, NightLightBackend, Observation, Schedule, TimeInfo},
};
use std::{
    collections::HashMap,
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{Registry::*, SystemInformation::*, Threading::*, Time::*},
    UI::{Accessibility::*, WindowsAndMessaging::*},
};
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub struct NativeClock {
    start: Instant,
}
impl Default for NativeClock {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}
impl Clock for NativeClock {
    fn now(&self) -> TimeInfo {
        // SAFETY: Valid output structures; conversion treats local civil time as UTC only to form an ordinal.
        let wall = unsafe {
            let mut st = zeroed();
            GetLocalTime(&mut st);
            let mut ft: FILETIME = zeroed();
            SystemTimeToFileTime(&st, &mut ft);
            ((u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)) / 600_000_000
        };
        TimeInfo {
            monotonic_ms: self.start.elapsed().as_millis() as u64,
            wall_minute: wall as i64,
            unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}
pub struct NativeNightLight {
    inner: win_nightlight_lib::Nightlight,
}
impl Default for NativeNightLight {
    fn default() -> Self {
        Self {
            inner: win_nightlight_lib::Nightlight::new(),
        }
    }
}
impl NightLightBackend for NativeNightLight {
    fn read(&mut self) -> Result<Observation, String> {
        let s = self.inner.read_state().map_err(|source| {
            crate::error::IntegrationError::NightLight {
                operation: "read state",
                source,
            }
            .to_string()
        })?;
        let settings = self.inner.read_settings().map_err(|source| {
            crate::error::IntegrationError::NightLight {
                operation: "read settings",
                source,
            }
            .to_string()
        })?;
        let minutes =
            |t: win_nightlight_lib::TimeOfDay| u16::from(t.hour()) * 60 + u16::from(t.minute());
        let (mode, start, end) = match settings.mode() {
            win_nightlight_lib::ScheduleMode::Off => ("Off", None, None),
            win_nightlight_lib::ScheduleMode::SetHours => (
                "SetHours",
                Some(minutes(settings.schedule_start())),
                Some(minutes(settings.schedule_end())),
            ),
            win_nightlight_lib::ScheduleMode::SunsetToSunrise => (
                "SunsetToSunrise",
                Some(minutes(settings.sunset())),
                Some(minutes(settings.sunrise())),
            ),
        };
        Ok(Observation {
            active: s.is_active(),
            usable: s.is_usable(),
            scheduled: s.transition_cause() == win_nightlight_lib::TransitionCause::Scheduled,
            fingerprint: s.to_bytes().map_err(|e| e.to_string())?,
            schedule: Schedule {
                mode: mode.into(),
                start,
                end,
            },
            preview: settings.is_previewing(),
        })
    }
    fn set_active(&mut self, active: bool, expected: &[u8]) -> Result<Observation, String> {
        let current = self.read()?;
        if current.fingerprint != expected {
            return Err("Night Light changed before write; operation cancelled to preserve external changes".into());
        }
        let mut matched = false;
        self.inner
            .update_state(|state| {
                if state.to_bytes().is_ok_and(|bytes| bytes == expected) {
                    matched = true;
                    state.set_active(active);
                }
            })
            .map_err(|source| {
                crate::error::IntegrationError::NightLight {
                    operation: "update state",
                    source,
                }
                .to_string()
            })?;
        if !matched {
            return Err(crate::error::IntegrationError::ConcurrentChange.to_string());
        }
        let result = self.read()?;
        if result.active != active {
            return Err(crate::error::IntegrationError::Verification.to_string());
        }
        Ok(result)
    }
}

#[derive(Default)]
pub struct NativeWindows {
    cache: HashMap<u32, (u64, Option<String>, Instant)>,
    pub enumerate_all: bool,
}
impl NativeWindows {
    fn identity(&mut self, pid: u32) -> Option<String> {
        if let Some((_, path, at)) = self.cache.get(&pid)
            && at.elapsed().as_secs() < 2
        {
            return path.clone();
        }
        // SAFETY: Limited query handle is closed on every path; output buffers are sized explicitly.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut created: FILETIME = zeroed();
            let (mut exit, mut kernel, mut user) = (zeroed(), zeroed(), zeroed());
            let token =
                if GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) != 0 {
                    (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)
                } else {
                    0
                };
            let mut buffer = vec![0u16; 32768];
            let mut len = buffer.len() as u32;
            let path = if QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut len) != 0
            {
                Some(String::from_utf16_lossy(&buffer[..len as usize]))
            } else {
                None
            };
            CloseHandle(handle);
            self.cache
                .insert(pid, (token, path.clone(), Instant::now()));
            path
        }
    }
    fn inspect(&mut self, hwnd: HWND, foreground: HWND) -> Option<Window> {
        // SAFETY: Window handles come from EnumWindows; all calls tolerate a concurrently destroyed window.
        unsafe {
            if IsWindowVisible(hwnd) == 0 {
                return None;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let mut class = [0u16; 256];
            let n = GetClassNameW(hwnd, class.as_mut_ptr(), 256);
            let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
            let shell = pid == std::process::id()
                || hwnd == GetDesktopWindow()
                || hwnd == GetShellWindow()
                || matches!(
                    class.as_str(),
                    "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
                );
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let incidental = ex & (WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT) != 0
                || matches!(
                    class.as_str(),
                    "tooltips_class32"
                        | "#32768"
                        | "Windows.UI.Core.CoreWindow"
                        | "XamlExplorerHostIslandWindow"
                ) && pid == 0;
            let mut bounds: RECT = zeroed();
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                (&mut bounds as *mut RECT).cast(),
                size_of::<RECT>() as u32,
            ) < 0
                && GetWindowRect(hwnd, &mut bounds) == 0
            {
                return None;
            }
            let mut cloaked = 0u32;
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED as u32,
                (&mut cloaked as *mut u32).cast(),
                4,
            );
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
            if monitor.is_null() {
                return None;
            }
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            let path = if shell { None } else { self.identity(pid) };
            let name = path
                .as_deref()
                .and_then(|p| std::path::Path::new(p).file_name())
                .map(|n| n.to_string_lossy().into_owned());
            let incidental = incidental
                || name.as_deref().is_some_and(|n| {
                    matches!(
                        n.to_ascii_lowercase().as_str(),
                        "shellexperiencehost.exe"
                            | "startmenuexperiencehost.exe"
                            | "searchhost.exe"
                            | "textinputhost.exe"
                            | "lockapp.exe"
                    )
                });
            let mut alpha = 255u8;
            let mut color = 0;
            let mut flags = 0;
            let transparent = ex & WS_EX_LAYERED != 0
                && GetLayeredWindowAttributes(hwnd, &mut color, &mut alpha, &mut flags) != 0
                && flags & LWA_ALPHA != 0
                && alpha == 0;
            Some(Window {
                id: hwnd as usize,
                foreground: hwnd == foreground,
                bounds: rect(bounds),
                monitor: rect(info.rcMonitor),
                visible: !transparent,
                minimized: IsIconic(hwnd) != 0,
                cloaked: cloaked != 0,
                shell,
                framed: style & (WS_CAPTION | WS_THICKFRAME) != 0,
                maximized: IsZoomed(hwnd) != 0,
                opaque: ex & (WS_EX_LAYERED | WS_EX_TRANSPARENT) == 0,
                incidental,
                path,
                name,
            })
        }
    }
}
fn rect(r: RECT) -> Rect {
    Rect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}
impl WindowSource for NativeWindows {
    fn snapshot(&mut self) -> Result<Vec<Window>, String> {
        if !self.enumerate_all {
            let foreground = unsafe { GetForegroundWindow() };
            return Ok(self.inspect(foreground, foreground).into_iter().collect());
        }
        unsafe extern "system" fn enumerate(hwnd: HWND, data: LPARAM) -> windows_sys::core::BOOL {
            // SAFETY: data points to the local vector for the duration of synchronous enumeration.
            unsafe {
                (&mut *(data as *mut Vec<HWND>)).push(hwnd);
            }
            1
        }
        let mut handles: Vec<HWND> = Vec::new();
        // SAFETY: Callback receives a valid local vector; enumeration is synchronous.
        unsafe {
            if EnumWindows(Some(enumerate), &mut handles as *mut _ as LPARAM) == 0 {
                return Err(format!(
                    "Enumerate windows: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let foreground = GetForegroundWindow();
            self.cache
                .retain(|_, (_, _, at)| at.elapsed().as_secs() < 120);
            Ok(handles
                .into_iter()
                .filter_map(|h| self.inspect(h, foreground))
                .collect())
        }
    }
}

pub fn set_startup(enabled: bool) -> Result<(), String> {
    // SAFETY: Registry handles and UTF-16 buffers stay valid through each synchronous call.
    unsafe {
        let mut key = null_mut();
        let path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let code = RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key);
        if code != 0 {
            return Err(format!("Open per-user startup key: Windows error {code}"));
        }
        let name = wide("FullscreenNightLight");
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let command = wide(&format!("\"{}\" --startup", exe.display()));
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key, name.as_ptr())
        };
        RegCloseKey(key);
        if result != 0 && !(result == ERROR_FILE_NOT_FOUND && !enabled) {
            return Err(format!(
                "Update startup registration: Windows error {result}"
            ));
        }
        Ok(())
    }
}
pub struct EventHooks {
    hooks: Vec<HWINEVENTHOOK>,
}
impl EventHooks {
    pub fn install(callback: WINEVENTPROC) -> Result<Self, String> {
        let mut hooks = Vec::new();
        for (first, last) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE),
            (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
        ] {
            // SAFETY: Out-of-context callback remains valid until hooks drop on this message-loop thread.
            let hook = unsafe {
                SetWinEventHook(
                    first,
                    last,
                    null_mut(),
                    callback,
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            if hook.is_null() {
                for h in hooks {
                    unsafe {
                        UnhookWinEvent(h);
                    }
                }
                return Err(format!(
                    "Install window event hook: {}",
                    std::io::Error::last_os_error()
                ));
            }
            hooks.push(hook);
        }
        Ok(Self { hooks })
    }
}
impl Drop for EventHooks {
    fn drop(&mut self) {
        for h in &self.hooks {
            unsafe {
                UnhookWinEvent(*h);
            }
        }
    }
}
pub fn open(target: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let verb = wide("open");
    let target = wide(target);
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err(format!(
            "Open failed (ShellExecute code {})",
            result as isize
        ))
    } else {
        Ok(())
    }
}
/// # Safety
/// `hwnd` must identify a live window owned by the calling GUI thread.
pub unsafe fn dwm_dark(hwnd: HWND, dark: bool) {
    let value: i32 = dark.into();
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&value as *const i32).cast::<c_void>(),
            4,
        );
    }
}
