//! GPUI assets: frontend branding plus shared semantic icons and fonts.

use std::borrow::Cow;

use gpui_kit::{App, AssetSource, Result, SharedString};
use volna_core::icons::{FONTS, IconName};

gpui_kit::assets::icon_assets!(
    ComponentAssets,
    [
        Check,
        Link,
        Unlink,
        Scan,
        Crosshair,
        Plus,
        Minus,
        X,
        Ellipsis,
        Maximize,
        Minimize,
        Pin,
        ChevronLeft
    ]
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = crate::ui::branding::ASSETS
            .iter()
            .find(|(name, _)| *name == path)
        {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        let bytes = match IconName::from_path(path) {
            Some(icon) => Some(Cow::Borrowed(icon.svg())),
            None => ComponentAssets.load(path)?,
        };
        let Some(bytes) = bytes else { return Ok(None) };
        // Keep bundled and toolkit icons at a consistent 1.5-unit stroke weight.
        let svg = std::str::from_utf8(&bytes)?
            .replace("stroke-width=\"2\"", "stroke-width=\"1.5\"")
            .replace("stroke-width=\"1.75\"", "stroke-width=\"1.5\"")
            .replace("stroke-width=\"1.7\"", "stroke-width=\"1.5\"");
        Ok(Some(Cow::Owned(svg.into_bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = ComponentAssets.list(path)?;
        paths.extend(
            crate::ui::branding::ASSETS
                .iter()
                .map(|(name, _)| *name)
                .filter(|name| name.starts_with(path))
                .map(SharedString::from),
        );
        paths.extend(
            IconName::ALL
                .iter()
                .map(|icon| icon.path())
                .filter(|p| p.starts_with(path))
                .map(SharedString::from),
        );
        Ok(paths)
    }
}

pub(crate) fn load_fonts(cx: &App) -> anyhow::Result<()> {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(f.bytes)).collect())
}
