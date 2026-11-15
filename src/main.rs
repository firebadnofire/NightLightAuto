// SPDX-License-Identifier: MPL-2.0
#![windows_subsystem = "windows"]
fn main() {
    if let Err(error) = fullscreen_night_light::gui::run() {
        let title = fullscreen_night_light::platform::wide("Fullscreen Night Light");
        let text = fullscreen_night_light::platform::wide(&error);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
}
