//! Connected monitors and the taskbar on each (Win32). Ids are the monitor device path from
//! `QueryDisplayConfig` (stable across restarts and re-plugging, unlike `\\.\DISPLAY2`), with the
//! GDI device name as fallback. The main taskbar is `Shell_TrayWnd` (it holds the tray icons);
//! other monitors have a `Shell_SecondaryTrayWnd` when Windows shows the taskbar on all displays.
use std::collections::HashMap;

use subhub_core::placement::{MonitorInfo, Rect};
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MONITORINFOEXW, MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowRect, IsWindowVisible,
};
use windows::core::{BOOL, w};

#[derive(Debug, Clone)]
pub struct Monitor {
    pub id: String,
    /// Friendly name ("DELL U2720Q"), else "Display".
    pub name: String,
    /// 1-based, left to right.
    pub number: usize,
    pub is_main: bool,
    pub handle: HMONITOR,
    /// Scale of the monitor (DPI / 96).
    pub scale: f32,
    /// The taskbar on this monitor, when Windows shows a horizontal one there.
    pub taskbar: Option<HWND>,
}

impl Monitor {
    pub fn label(&self) -> String {
        let main = if self.is_main { " (main)" } else { "" };
        format!("{} · {}{main}", self.number, self.name)
    }

    pub fn info(&self) -> MonitorInfo {
        MonitorInfo {
            id: self.id.clone(),
            is_main: self.is_main,
            has_taskbar: self.taskbar.is_some(),
        }
    }

    /// The taskbar's rect (physical px).
    pub fn taskbar_rect(&self) -> Option<RECT> {
        let mut r = RECT::default();
        unsafe { GetWindowRect(self.taskbar?, &mut r).ok()? };
        Some(r)
    }
}

pub fn rect(r: RECT) -> Rect {
    Rect {
        left: r.left as f32,
        top: r.top as f32,
        right: r.right as f32,
        bottom: r.bottom as f32,
    }
}

fn wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// GDI device name (`\\.\DISPLAY1`) -> (device path, friendly name), from the active paths.
fn display_names() -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return out;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut np,
            paths.as_mut_ptr(),
            &mut nm,
            modes.as_mut_ptr(),
            None,
        ) != ERROR_SUCCESS
        {
            return out;
        }
        for p in &paths[..np as usize] {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = p.sourceInfo.adapterId;
            source.header.id = p.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            target.header.adapterId = p.targetInfo.adapterId;
            target.header.id = p.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut target.header) != 0 {
                continue;
            }
            let path = wide(&target.monitorDevicePath);
            if path.is_empty() {
                continue;
            }
            out.entry(wide(&source.viewGdiDeviceName))
                .or_insert((path, wide(&target.monitorFriendlyDeviceName)));
        }
    }
    out
}

/// Visible, horizontal taskbars: (window, is the main one).
fn taskbars() -> Vec<(HWND, bool)> {
    let mut out = Vec::new();
    unsafe {
        if let Ok(main) = FindWindowW(w!("Shell_TrayWnd"), None) {
            out.push((main, true));
        }
        let mut prev: Option<HWND> = None;
        while let Ok(h) = FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), None) {
            out.push((h, false));
            prev = Some(h);
        }
    }
    out.retain(|(h, _)| unsafe {
        let mut r = RECT::default();
        IsWindowVisible(*h).as_bool()
            && GetWindowRect(*h, &mut r).is_ok()
            && r.bottom > r.top
            && (r.right - r.left) >= (r.bottom - r.top) * 3
    });
    out
}

unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    unsafe { (*(data.0 as *mut Vec<HMONITOR>)).push(m) };
    true.into()
}

/// GPUI's id for a monitor (its HMONITOR). Windows must be opened on their monitor: moving
/// one onto another monitor inside a GPUI update re-enters GPUI and panics.
pub fn display_id(m: HMONITOR) -> gpui::DisplayId {
    gpui::DisplayId::new(m.0 as u64)
}

pub fn scale_of(m: HMONITOR) -> f32 {
    let (mut x, mut y) = (96u32, 96u32);
    if unsafe { GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut x, &mut y) }.is_ok() {
        x as f32 / 96.0
    } else {
        1.0
    }
}

/// GPUI display, scale and work area (physical px) of the monitor at a physical point.
pub fn at_point(x: i32, y: i32) -> Option<(gpui::DisplayId, f32, RECT)> {
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(m, &mut info).ok().ok()?;
        Some((display_id(m), scale_of(m), info.rcWork))
    }
}

/// All connected monitors, numbered left to right.
pub fn list() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut _ as isize),
        );
    }
    let names = display_names();
    let bars: Vec<(HMONITOR, HWND, bool)> = taskbars()
        .into_iter()
        .map(|(h, main)| {
            (
                unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) },
                h,
                main,
            )
        })
        .collect();
    let mut out: Vec<(i32, Monitor)> = handles
        .into_iter()
        .filter_map(|m| {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            unsafe { GetMonitorInfoW(m, &mut info.monitorInfo).ok().ok()? };
            let gdi = wide(&info.szDevice);
            let (id, name) = names
                .get(&gdi)
                .cloned()
                .unwrap_or_else(|| (gdi.clone(), String::new()));
            let bar = bars.iter().find(|(bm, _, _)| *bm == m);
            Some((
                info.monitorInfo.rcMonitor.left,
                Monitor {
                    id,
                    name: if name.is_empty() {
                        "Display".into()
                    } else {
                        name
                    },
                    number: 0,
                    is_main: bar.is_some_and(|(_, _, main)| *main),
                    handle: m,
                    scale: scale_of(m),
                    taskbar: bar.map(|(_, h, _)| *h),
                },
            ))
        })
        .collect();
    out.sort_by_key(|(left, _)| *left);
    out.into_iter()
        .enumerate()
        .map(|(i, (_, mut m))| {
            m.number = i + 1;
            m
        })
        .collect()
}

/// The monitor with this id.
pub fn find(id: &str) -> Option<Monitor> {
    list().into_iter().find(|m| m.id == id)
}
