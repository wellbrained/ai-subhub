//! Where things sit on each taskbar, read through UI Automation (owner, 2026-11-08: the badges
//! used fixed guesses). Windows 11 draws its taskbar in XAML: the main taskbar still has a
//! `TrayNotifyWnd` window to measure, but the clock on other monitors' taskbars and the widgets
//! (weather) button are only visible to UI Automation. A background thread reads them every
//! `EVERY` and keeps the result here, so placing the badges never waits on Explorer.
use std::collections::HashMap;
use std::sync::{Mutex, Once};
use std::time::Duration;

use windows::Win32::Foundation::HWND;

/// How often the taskbars are read again (icons come and go, the clock widens).
const EVERY: Duration = Duration::from_secs(10);

/// Measured parts of one taskbar, physical px.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Layout {
    /// Left edge of the tray area (hidden-icons arrow, tray icons, clock), whatever is leftmost.
    pub tray_left: Option<i32>,
    /// Right edge of the widgets button, when the taskbar shows one.
    pub widgets_right: Option<i32>,
}

/// One UI Automation element: class name, automation id, left and right edge.
pub type Part<'a> = (&'a str, &'a str, i32, i32);

/// The layout from a taskbar's elements. Tray parts are the `SystemTray.*` classes (arrow,
/// icons, input indicator, network, volume, clock, show desktop).
pub fn layout_of(parts: &[Part]) -> Layout {
    let tray_left = parts
        .iter()
        .filter(|(class, _, l, r)| class.starts_with("SystemTray.") && r > l)
        .map(|(_, _, l, _)| *l)
        .min();
    let widgets_right = parts
        .iter()
        .find(|(_, id, l, r)| *id == "WidgetsButton" && r > l)
        .map(|(_, _, _, r)| *r);
    Layout {
        tray_left,
        widgets_right,
    }
}

static CACHE: Mutex<Option<HashMap<isize, Layout>>> = Mutex::new(None);
static START: Once = Once::new();

/// Starts the reader thread (once).
pub fn start() {
    START.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("taskbar-layout".into())
            .spawn(|| {
                let Some(uia) = automation() else { return };
                loop {
                    let fresh = read_all(&uia);
                    if let Ok(mut c) = CACHE.lock() {
                        *c = Some(fresh);
                    }
                    std::thread::sleep(EVERY);
                }
            });
    });
}

/// The last read layout of the taskbar window `bar` (None until the first read finished).
pub fn get(bar: HWND) -> Option<Layout> {
    let c = CACHE.lock().ok()?;
    c.as_ref()?.get(&(bar.0 as isize)).copied()
}

fn automation() -> Option<windows::Win32::UI::Accessibility::IUIAutomation> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Accessibility::CUIAutomation;
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    }
}

/// All taskbars: the main one and each other monitor's.
fn bars() -> Vec<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW};
    use windows::core::w;
    let mut out = Vec::new();
    unsafe {
        if let Ok(main) = FindWindowW(w!("Shell_TrayWnd"), None) {
            out.push(main);
        }
        let mut prev = None;
        while let Ok(h) = FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), None) {
            out.push(h);
            prev = Some(h);
        }
    }
    out
}

fn read_all(uia: &windows::Win32::UI::Accessibility::IUIAutomation) -> HashMap<isize, Layout> {
    use windows::Win32::UI::Accessibility::TreeScope_Descendants;
    let mut out = HashMap::new();
    for bar in bars() {
        let parts = unsafe {
            let Ok(root) = uia.ElementFromHandle(bar) else {
                continue;
            };
            let Ok(cond) = uia.CreateTrueCondition() else {
                continue;
            };
            let Ok(all) = root.FindAll(TreeScope_Descendants, &cond) else {
                continue;
            };
            let n = all.Length().unwrap_or(0);
            let mut parts: Vec<(String, String, i32, i32)> = Vec::new();
            for i in 0..n {
                let Ok(e) = all.GetElement(i) else { continue };
                let class = e
                    .CurrentClassName()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let id = e
                    .CurrentAutomationId()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if !class.starts_with("SystemTray.") && id != "WidgetsButton" {
                    continue;
                }
                if let Ok(r) = e.CurrentBoundingRectangle() {
                    parts.push((class, id, r.left, r.right));
                }
            }
            parts
        };
        let view: Vec<Part> = parts
            .iter()
            .map(|(c, i, l, r)| (c.as_str(), i.as_str(), *l, *r))
            .collect();
        out.insert(bar.0 as isize, layout_of(&view));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_taskbar_tray_starts_at_the_hidden_icons_arrow() {
        // As read on the owner's PC (2560 px wide main taskbar, 2026-11-08).
        let parts = [
            ("ToggleButton", "WidgetsButton", 10, 54),
            ("ToggleButton", "StartButton", 1060, 1105),
            ("SystemTray.NormalButton", "SystemTrayIcon", 2206, 2238),
            ("SystemTray.NormalButton", "NotifyItemIcon", 2238, 2270),
            ("SystemTray.OmniButton", "SystemTrayIcon", 2474, 2544),
            ("SystemTray.ShowDesktopButton", "SystemTrayIcon", 2548, 2560),
        ];
        assert_eq!(
            layout_of(&parts),
            Layout {
                tray_left: Some(2206),
                widgets_right: Some(54)
            }
        );
    }

    #[test]
    fn other_taskbars_have_only_the_clock_and_empty_rects_are_skipped() {
        let parts = [
            ("SystemTray.OmniButton", "SystemTrayIcon", -78, -8),
            ("SystemTray.NormalButton", "x", -500, -500),
        ];
        let l = layout_of(&parts);
        assert_eq!(l.tray_left, Some(-78));
        assert_eq!(l.widgets_right, None);
        assert_eq!(layout_of(&[]), Layout::default());
    }
}
