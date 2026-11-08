//! Keeps the taskbar displays on top the moment another window comes to the front (owner,
//! 2026-11-08: the badges flickered when switching windows). Clicking the taskbar or switching
//! apps can put the taskbar above them; the display's 1 s loop lifted them back only on its next
//! tick. A WinEvent hook on foreground changes re-raises them at once. Hidden displays (stepped
//! aside for a fullscreen app) stay hidden: `SetWindowPos` without `SWP_SHOWWINDOW`.
use std::sync::{Mutex, Once};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, HWND_TOPMOST, IsWindow, IsWindowVisible, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SetWindowPos, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

/// Display windows to keep on top (raw handles; only touched on the UI thread).
static DISPLAYS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static HOOK: Once = Once::new();

/// Keeps `hwnd` on top from now on (installs the hook on first use; UI thread only, since the
/// hook's events arrive through that thread's message loop).
pub fn track(hwnd: HWND) {
    HOOK.call_once(|| unsafe {
        let _ = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
    });
    if let Ok(mut list) = DISPLAYS.lock() {
        let raw = hwnd.0 as isize;
        if !list.contains(&raw) {
            list.push(raw);
        }
    }
}

/// Stops keeping `hwnd` on top (its display closed).
pub fn untrack(hwnd: HWND) {
    if let Ok(mut list) = DISPLAYS.lock() {
        list.retain(|h| *h != hwnd.0 as isize);
    }
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let Ok(list) = DISPLAYS.lock() else { return };
    for raw in list.iter() {
        let hwnd = HWND(*raw as *mut _);
        unsafe {
            if IsWindow(Some(hwnd)).as_bool() && IsWindowVisible(hwnd).as_bool() {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
}
