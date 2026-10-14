//! Settings declarations, layered resolution, editing, search, and generated schemas.

mod controller;
pub mod jsonc;
pub mod registry;
pub mod schema;
pub mod search;
pub mod store;

pub use registry::{
    Apply, Choice, Host, Hosts, Kind, MAX_MEMORY_MIB, Page, REGISTRY, Spec, ZOOM_MAX, ZOOM_MIN,
    ZOOM_STEP, spec,
};
pub use search::{Hit, Matched, search};
pub use store::{Diagnostic, Severity, Store, WRITE_IDLE};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Integer(i64),
    Number(f64),
    Text(String),
}

impl Value {
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Integer(i) => serde_json::Value::from(*i),
            Value::Number(n) => serde_json::Value::from(*n),
            Value::Text(s) => serde_json::Value::String(s.clone()),
        }
    }

    #[must_use]
    pub fn to_json_text(&self) -> String {
        self.to_json().to_string()
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Integer(i)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_owned())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Animation {
    #[default]
    On,
    Reduced,
    Off,
}

impl Animation {
    #[must_use]
    pub fn duration(self) -> Option<std::time::Duration> {
        match self {
            Animation::On => Some(std::time::Duration::from_millis(140)),
            Animation::Reduced => Some(std::time::Duration::from_millis(70)),
            Animation::Off => None,
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "reduced" => Animation::Reduced,
            "off" => Animation::Off,
            _ => Animation::On,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Autosave {
    #[default]
    Sidecar,
    Vscode,
    Off,
}

impl Autosave {
    fn parse(s: &str) -> Self {
        match s {
            "vscode" => Autosave::Vscode,
            "off" => Autosave::Off,
            _ => Autosave::Sidecar,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppearanceSettings {
    pub theme: String,
    pub zoom: f64,
    pub digital_wave_width: f64,
    pub analog_wave_width: f64,
    pub wave_spacing: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomStep {
    In,
    Out,
    Reset,
}

impl ZoomStep {
    #[must_use]
    pub fn apply(self, current: f64) -> f64 {
        let next = match self {
            ZoomStep::In => current + ZOOM_STEP,
            ZoomStep::Out => current - ZOOM_STEP,
            ZoomStep::Reset => 1.0,
        };
        normalize_zoom(next)
    }
}

fn normalize_zoom(zoom: f64) -> f64 {
    let zoom = if zoom.is_finite() { zoom } else { 1.0 };
    let steps = (zoom / ZOOM_STEP).round();
    ((steps * ZOOM_STEP * 10.0).round() / 10.0).clamp(ZOOM_MIN, ZOOM_MAX)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelSettings {
    pub link_by_default: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaveSettings {
    pub animation: Animation,
    pub snap_pixels: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSettings {
    pub autosave: Autosave,
    pub recent_limit: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionSettings {
    pub detail_items: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemorySettings {
    pub budget_mib: u64,
    pub object_mib: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSettings {
    pub server_path: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityIndexPolicy {
    #[default]
    Ask,
    Always,
    Never,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HierarchySettings {
    pub activity_index: ActivityIndexPolicy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub appearance: AppearanceSettings,
    pub panels: PanelSettings,
    pub waves: WaveSettings,
    pub workspace: WorkspaceSettings,
    pub transaction: TransactionSettings,
    pub memory: MemorySettings,
    pub remote: RemoteSettings,
    pub hierarchy: HierarchySettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_values(&|id| spec(id).expect("registered id").default.value())
    }
}

impl Settings {
    pub(super) fn from_values(value: &dyn Fn(&str) -> Value) -> Self {
        let text = |id: &str| value(id).as_str().unwrap_or_default().to_owned();
        let int = |id: &str| value(id).as_i64().unwrap_or_default();
        Settings {
            hierarchy: HierarchySettings {
                activity_index: match text("hierarchy.activityIndex").as_str() {
                    "always" => ActivityIndexPolicy::Always,
                    "never" => ActivityIndexPolicy::Never,
                    _ => ActivityIndexPolicy::Ask,
                },
            },
            appearance: AppearanceSettings {
                theme: text("appearance.theme"),
                zoom: normalize_zoom(value("appearance.zoom").as_f64().unwrap_or(1.0)),
                digital_wave_width: value("appearance.digitalWaveWidth").as_f64().unwrap_or(1.0),
                analog_wave_width: value("appearance.analogWaveWidth").as_f64().unwrap_or(1.25),
                wave_spacing: value("appearance.waveSpacing").as_f64().unwrap_or(5.0),
            },
            panels: PanelSettings {
                link_by_default: value("panels.linkByDefault").as_bool().unwrap_or(true),
            },
            waves: WaveSettings {
                animation: Animation::parse(&text("waves.animation")),
                snap_pixels: int("waves.snapPixels").max(0) as u32,
            },
            workspace: WorkspaceSettings {
                autosave: Autosave::parse(&text("workspace.autosave")),
                recent_limit: int("workspace.recentLimit").max(1) as usize,
            },
            transaction: TransactionSettings {
                detail_items: int("transaction.detailItems").clamp(1, 1000) as usize,
            },
            memory: MemorySettings {
                budget_mib: int("memory.budgetMiB").max(1) as u64,
                object_mib: int("memory.objectMiB").max(1) as u64,
            },
            remote: RemoteSettings {
                server_path: text("remote.serverPath"),
            },
        }
    }

    #[must_use]
    pub fn link_by_default(&self) -> crate::wave::model::Link {
        crate::wave::model::Link {
            viewport: self.panels.link_by_default,
            cursor: self.panels.link_by_default,
        }
    }

    #[must_use]
    pub fn limits(&self) -> volna_trace::remote::limits::Limits {
        volna_trace::remote::limits::Limits {
            memory_mib: self.memory.budget_mib,
            object_mib: self.memory.object_mib,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_clamp_round_and_reset() {
        assert_eq!(Settings::default().appearance.zoom, 1.0);
        assert_eq!(ZoomStep::In.apply(1.0), 1.1);
        assert_eq!(ZoomStep::In.apply(1.1), 1.2);
        assert_eq!(ZoomStep::Out.apply(1.0), 0.9);
        assert_eq!(ZoomStep::Out.apply(0.5), 0.5);
        assert_eq!(ZoomStep::In.apply(3.0), 3.0);
        assert_eq!(ZoomStep::Reset.apply(2.3), 1.0);
        let mut z = 0.5;
        for _ in 0..20 {
            z = ZoomStep::In.apply(z);
        }
        assert_eq!(z, 2.5);
        assert_eq!(normalize_zoom(0.4), ZOOM_MIN);
        assert_eq!(normalize_zoom(3.1), ZOOM_MAX);
        assert_eq!(normalize_zoom(f64::NAN), 1.0);
        let resolved = Settings::from_values(&|id| match id {
            "appearance.zoom" => Value::Number(1.75),
            _ => spec(id).unwrap().default.value(),
        });
        assert_eq!(resolved.appearance.zoom, 1.8);
        let resolved = Settings::from_values(&|id| match id {
            "appearance.zoom" => Value::Integer(2),
            _ => spec(id).unwrap().default.value(),
        });
        assert_eq!(resolved.appearance.zoom, 2.0);
    }
}
