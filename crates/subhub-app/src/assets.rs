//! Asset source: our own SVGs (provider marks) first, then the Lucide catalog that ships with
//! gpui-component (`gpui-kit-assets`), so UI icons share one consistent stroke.
use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

const OWN: [(&str, &[u8]); 2] = [
    (
        "icons/provider-claude.svg",
        include_bytes!("../assets/icons/provider-claude.svg"),
    ),
    (
        "icons/provider-openai.svg",
        include_bytes!("../assets/icons/provider-openai.svg"),
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
