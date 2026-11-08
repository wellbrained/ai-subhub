//! Asset source: our own SVGs (provider marks) first, then the Lucide catalog that ships with
//! gpui-component (`gpui-kit-assets`), so UI icons share one consistent stroke.
use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

const OWN: [(&str, &[u8]); 10] = [
    ("icons/app-logo.svg", crate::logo::APP_SVG),
    ("icons/app-logo-small.svg", crate::logo::APP_SMALL_SVG),
    ("icons/mark-dark.svg", crate::logo::TRAY_DARK_SVG),
    ("icons/mark-light.svg", crate::logo::TRAY_LIGHT_SVG),
    (
        "icons/provider-claude.svg",
        include_bytes!("../assets/icons/provider-claude.svg"),
    ),
    (
        "icons/provider-openai.svg",
        include_bytes!("../assets/icons/provider-openai.svg"),
    ),
    // Window controls of the merged title bar (10 px, 1 px strokes like Windows' own).
    (
        "icons/win-minimize.svg",
        include_bytes!("../assets/icons/win-minimize.svg"),
    ),
    (
        "icons/win-maximize.svg",
        include_bytes!("../assets/icons/win-maximize.svg"),
    ),
    (
        "icons/win-restore.svg",
        include_bytes!("../assets/icons/win-restore.svg"),
    ),
    (
        "icons/win-close.svg",
        include_bytes!("../assets/icons/win-close.svg"),
    ),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = OWN.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit_assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut v: Vec<SharedString> = OWN
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| (*p).into())
            .collect();
        v.extend(gpui_kit_assets::AllAssets.list(path)?);
        Ok(v)
    }
}
