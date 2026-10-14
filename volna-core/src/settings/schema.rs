//! Generated artefacts: the JSON Schema of `settings.json`, VS Code's
//! `contributes.configuration` block and the read-only default document.

use super::registry::{Apply, Host, REGISTRY};
use std::fmt::Write as _;

#[must_use]
pub fn json_schema(host: Host) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "$schema".into(),
        serde_json::json!({"type": "string", "description": "The schema of this file."}),
    );
    for spec in REGISTRY.iter().filter(|s| s.available(host)) {
        properties.insert(spec.id.into(), spec.schema(Some(host)));
    }
    serde_json::json!({
        "$schema": "https://json-schema.org/draft-07/schema",
        "title": "Volna user settings",
        "description": "Only the keys you change belong here; every other key uses its default.",
        "type": "object",
        "properties": properties,
        "additionalProperties": false,
    })
}

#[must_use]
pub fn vscode_configuration() -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    for spec in REGISTRY.iter().filter(|s| s.available(Host::Vscode)) {
        let mut property = spec.schema(Some(Host::Vscode));
        if spec.id == "remote.serverPath" {
            property["scope"] = "machine".into();
        } else if spec.apply == Apply::OnReopen {
            property["scope"] = "resource".into();
        }
        properties.insert(format!("volna.{}", spec.id), property);
    }
    serde_json::json!({
        "title": "Volna",
        "properties": properties,
    })
}

#[must_use]
pub fn default_document(host: Host) -> String {
    let mut out = String::from(
        "// Default settings. This document is generated and read-only;\n// copy a key into settings.json to change it.\n{\n",
    );
    let specs: Vec<_> = REGISTRY.iter().filter(|s| s.available(host)).collect();
    for (ix, spec) in specs.iter().enumerate() {
        for line in spec.description.split('\n') {
            writeln!(out, "  // {line}").unwrap();
        }
        if let Some(badge) = spec.apply.badge() {
            writeln!(out, "  // ({badge})").unwrap();
        }
        let comma = if ix + 1 < specs.len() { "," } else { "" };
        writeln!(
            out,
            "  \"{}\": {}{comma}",
            spec.id,
            spec.default.value().to_json_text()
        )
        .unwrap();
        if ix + 1 < specs.len() {
            out.push('\n');
        }
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_every_host_key_and_the_default_document_parses() {
        let schema = json_schema(Host::Native);
        let properties = schema["properties"].as_object().unwrap();
        assert!(properties.contains_key("waves.snapPixels"));
        assert!(!properties.contains_key("remote.serverPath"));
        assert_eq!(properties["memory.budgetMiB"]["maximum"], 262_144);
        assert_eq!(properties["waves.snapPixels"]["maximum"], 24);
        assert_eq!(
            properties["waves.animation"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let doc = super::super::jsonc::parse(&default_document(Host::Vscode)).unwrap();
        assert_eq!(
            doc.entries.len(),
            REGISTRY
                .iter()
                .filter(|s| s.available(Host::Vscode))
                .count()
        );
        assert_eq!(doc.get("memory.budgetMiB").unwrap().value, 512);
        let contribution = vscode_configuration();
        assert_eq!(
            contribution["properties"]["volna.remote.serverPath"]["scope"],
            "machine"
        );
        assert!(
            contribution["properties"]["volna.workspace.autosave"]["markdownDescription"]
                .as_str()
                .unwrap()
                .ends_with("Reopen the trace to apply.")
        );
        assert!(
            contribution["properties"]
                .get("volna.appearance.theme")
                .is_none()
        );
    }
}
