//! Themes selectable by `appearance.theme`: the Volna themes (the default), One Dark, and
//! bundled palettes.

use super::{HostPalette, Theme, Themes};

#[derive(Clone, Copy, Debug)]
pub struct Builtin {
    pub id: &'static str,
    pub label: &'static str,
    json: &'static str,
}

macro_rules! builtin {
    ($id:literal, $label:literal) => {
        Builtin {
            id: $id,
            label: $label,
            json: include_str!(concat!("../../assets/themes/", $id, ".json")),
        }
    };
}

pub const BUILTIN: &[Builtin] = &[
    builtin!("dracula", "Dracula"),
    builtin!("catppuccin-mocha", "Catppuccin Mocha"),
    builtin!("catppuccin-latte", "Catppuccin Latte"),
    builtin!("github-dark", "GitHub Dark"),
    builtin!("github-light", "GitHub Light"),
    builtin!("vscode-dark", "VS Code Dark Modern"),
    builtin!("vscode-light", "VS Code Light Modern"),
];

pub const VOLNA: &str = "volna";
pub const VOLNA_DARK: &str = "volna-dark";
pub const VOLNA_LIGHT: &str = "volna-light";
pub const VOLNA_MIXED: &str = "volna-mixed";
pub const VOLNA_ISLANDS_DARK: &str = "volna-islands-dark";
pub const ONE_DARK: &str = "one-dark";

const CODED: [(&str, &str); 6] = [
    (VOLNA, "Volna"),
    (VOLNA_DARK, "Volna Dark"),
    (VOLNA_ISLANDS_DARK, "Volna Islands Dark"),
    (VOLNA_LIGHT, "Volna Light"),
    (VOLNA_MIXED, "Volna Mixed"),
    (ONE_DARK, "One Dark"),
];

pub fn choices() -> impl Iterator<Item = (&'static str, &'static str)> {
    CODED
        .into_iter()
        .chain(BUILTIN.iter().map(|b| (b.id, b.label)))
}

#[must_use]
pub fn is_builtin(id: &str) -> bool {
    choices().any(|(known, _)| known == id)
}

#[must_use]
pub fn resolve(id: &str, system_dark: bool) -> Option<Themes> {
    let volna = Theme::volna;
    Some(match id {
        VOLNA => Themes::uniform(volna(system_dark)),
        VOLNA_ISLANDS_DARK => Themes::uniform(Theme::islands_dark()),
        VOLNA_DARK => Themes::uniform(volna(true)),
        VOLNA_LIGHT => Themes::uniform(volna(false)),
        VOLNA_MIXED => Themes {
            chrome: volna(false),
            canvas: volna(true),
        },
        ONE_DARK => Themes::uniform(Theme::one_dark()),
        _ => {
            let builtin = BUILTIN.iter().find(|b| b.id == id)?;
            let palette = HostPalette::from_json(builtin.json).expect("bundled palettes are valid");
            Themes::uniform(Theme::from_host(&palette))
        }
    })
}

impl Theme {
    #[must_use]
    pub fn builtin(id: &str) -> Option<Self> {
        resolve(id, true).map(|themes| themes.chrome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Appearance;

    #[test]
    fn islands_is_opt_in_and_preserves_data_semantics_at_every_zoom() {
        let dark = resolve(VOLNA_DARK, true).unwrap().chrome;
        for system_dark in [false, true] {
            let islands = resolve(VOLNA_ISLANDS_DARK, system_dark).unwrap();
            for zoom in [0.75, 1.0, 1.25, 2.0] {
                let t = islands.chrome.zoomed(zoom);
                let style = t.islands.unwrap();
                assert!(super::super::contrast(t.panel.bg, style.background) >= 1.2);
                assert!(style.background.l > t.panel.bg.l);
                for background in style.ocean {
                    assert!(super::super::contrast(t.bar.text, background) >= 4.5);
                    assert!(super::super::contrast(t.bar.text_muted, background) >= 4.5);
                    assert!(super::super::contrast(t.bar.icon, background) >= 3.0);
                }
                assert_eq!(t.panel.bg, t.editor.bg);
                assert_eq!(t.bar.bg, t.panel.bg);
                assert_eq!(t.wave_signal, dark.wave_signal);
                assert_eq!(t.wave_undef, dark.wave_undef);
                assert_eq!(t.wave_highimp, dark.wave_highimp);
                assert_eq!(t.wave_dontcare, dark.wave_dontcare);
                assert_eq!(t.stages, dark.stages);
                for (a, b) in t.markers.iter().zip(dark.markers) {
                    assert_eq!(a.stroke, b.stroke);
                    assert_eq!(a.background, b.background);
                }
                let reset = t.zoomed(1.0);
                assert_eq!(reset.scale(style.gap), style.gap);
                assert_eq!(reset.row_height, dark.row_height);
            }
            for (id, _) in choices().filter(|(id, _)| *id != VOLNA_ISLANDS_DARK) {
                let themes = resolve(id, system_dark).unwrap();
                assert!(themes.chrome.islands.is_none(), "{id}");
                assert!(themes.canvas.islands.is_none(), "{id}");
            }
        }
    }

    #[test]
    fn every_bundled_palette_resolves_with_its_declared_appearance() {
        let mut ids = std::collections::HashSet::new();
        for builtin in BUILTIN {
            assert!(ids.insert(builtin.id), "duplicate id {}", builtin.id);
            let theme = Theme::builtin(builtin.id).unwrap();
            let light = builtin.id.ends_with("light") || builtin.id.ends_with("latte");
            assert_eq!(
                theme.appearance,
                if light {
                    Appearance::Light
                } else {
                    Appearance::Dark
                },
                "{}",
                builtin.id
            );
            assert_ne!(theme.editor.bg, theme.bar.bg, "{}", builtin.id);
            assert!(is_builtin(builtin.id));
        }
        assert!(Theme::builtin("one-dark").is_some());
        assert!(Theme::builtin("nope").is_none());
        assert_eq!(choices().count(), BUILTIN.len() + CODED.len());
    }

    #[test]
    fn volna_follows_the_system_and_mixed_is_a_light_window_with_dark_data_panels() {
        let dark = |t: &Theme| t.appearance.is_dark();
        for system_dark in [true, false] {
            let volna = resolve(VOLNA, system_dark).unwrap();
            assert_eq!(dark(&volna.chrome), system_dark);
            assert_eq!(volna.canvas.editor.bg, volna.chrome.editor.bg);
            let mixed = resolve(VOLNA_MIXED, system_dark).unwrap();
            assert!(!dark(&mixed.chrome), "Mixed ignores the system");
            assert!(dark(&mixed.canvas));
            let pinned = resolve(VOLNA_DARK, system_dark).unwrap();
            assert_eq!(mixed.canvas.wave_signal, pinned.canvas.wave_signal);
            assert_eq!(mixed.canvas.editor.bg, pinned.canvas.editor.bg);
            assert!(dark(&pinned.chrome));
            assert!(!dark(&resolve(VOLNA_LIGHT, system_dark).unwrap().chrome));
        }
        for (id, _) in choices() {
            assert!(resolve(id, false).is_some(), "{id}");
        }
        assert!(resolve("volna-sepia", true).is_none());
    }
}
