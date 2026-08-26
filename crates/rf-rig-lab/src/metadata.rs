//! Renders the package metadata from the contract.
//!
//! `metadata/parameters.json`, `metadata/presets.json` and
//! `metadata/runtime.json` are generated, never edited. The contract crate is
//! the single source of truth for what a parameter is called, what range it
//! accepts and which pedal owns it, so the host's schema and the engine's
//! behaviour cannot disagree.

use std::path::Path;

use rf_rig_contract::{Control, Kind, PAGES, PARAMETERS, PEDALS, PRESETS};
use serde_json::{Map, Value, json};

use crate::manifest::PackageIdentity;

pub const PARAMETER_SCHEMA_VERSION: u32 = 1;
pub const PRESET_CATALOG_SCHEMA_VERSION: u32 = 1;
pub const RUNTIME_SCHEMA_VERSION: u32 = 1;

pub fn parameter_schema() -> Value {
    let pages: Vec<Value> = PAGES
        .iter()
        .map(|page| {
            let mut descriptor = Map::new();
            descriptor.insert("id".into(), json!(page.id));
            descriptor.insert("name".into(), json!(page.name));
            descriptor.insert("order".into(), json!(page.order));
            // The page header carries the circuit family this pedal models.
            // Publishing it through the schema is what lets the web surface
            // describe a pedal without keeping its own copy of the text.
            if let Some(pedal) = PEDALS.iter().find(|pedal| pedal.page == page.id) {
                descriptor.insert("header".into(), json!(pedal.circuit));
            }
            Value::Object(descriptor)
        })
        .collect();

    let parameters: Vec<Value> = PARAMETERS
        .iter()
        .map(|parameter| {
            let mut descriptor = Map::new();
            descriptor.insert("index".into(), json!(parameter.index));
            descriptor.insert("id".into(), json!(parameter.id));
            descriptor.insert("name".into(), json!(parameter.name));
            descriptor.insert("page".into(), json!(parameter.page));
            descriptor.insert("order".into(), json!(parameter.order));
            descriptor.insert("kind".into(), kind(&parameter.kind));
            descriptor.insert(
                "flags".into(),
                json!({
                    "automatable": true,
                    // Only a continuous control is worth handing to a modulation
                    // source; a switch or a selector is not.
                    "modulatable": matches!(parameter.kind, Kind::Float { .. }),
                    "read_only": false,
                    "advanced": matches!(parameter.kind, Kind::Integer { .. }),
                }),
            );
            descriptor.insert(
                "suggested_control".into(),
                json!(control(parameter.control)),
            );
            Value::Object(descriptor)
        })
        .collect();

    json!({
        "schema_version": PARAMETER_SCHEMA_VERSION,
        "pages": pages,
        "parameters": parameters,
    })
}

/// Widens an `f32` without dragging its binary tail into the JSON. Printing
/// the shortest representation that round-trips and re-reading it keeps the
/// document readable: `0.001`, not `0.0010000000474974513`.
fn number(value: f32) -> Value {
    let widened: f64 = format!("{value}").parse().unwrap_or(value as f64);
    json!(widened)
}

fn kind(kind: &Kind) -> Value {
    match *kind {
        Kind::Float {
            minimum,
            maximum,
            default,
            step,
            unit,
        } => {
            let mut value = Map::new();
            value.insert("type".into(), json!("float"));
            value.insert("minimum".into(), number(minimum));
            value.insert("maximum".into(), number(maximum));
            value.insert("default".into(), number(default));
            value.insert("step".into(), number(step));
            if let Some(unit) = unit {
                value.insert("unit".into(), json!(unit));
            }
            Value::Object(value)
        }
        Kind::Boolean { default } => json!({ "type": "boolean", "default": default }),
        Kind::Integer {
            minimum,
            maximum,
            default,
        } => json!({
            "type": "integer",
            "minimum": minimum,
            "maximum": maximum,
            "default": default,
            "step": 1,
        }),
        Kind::Enum { default, choices } => {
            let choices: Vec<Value> = choices
                .iter()
                .enumerate()
                .map(|(value, name)| json!({ "value": value, "name": name }))
                .collect();
            json!({ "type": "enum", "default": default, "choices": choices })
        }
    }
}

fn control(control: Control) -> &'static str {
    match control {
        Control::Knob => "knob",
        Control::Toggle => "toggle",
        Control::List => "list",
    }
}

pub fn preset_catalog() -> Value {
    let presets: Vec<Value> = PRESETS
        .iter()
        .enumerate()
        .map(|(order, preset)| {
            json!({
                "id": preset.id,
                "name": preset.name,
                "description": preset.description,
                "bank": "factory",
                "category": "Board",
                "order": order as i32,
                "tags": [],
                "editable": false,
            })
        })
        .collect();

    json!({
        "schema_version": PRESET_CATALOG_SCHEMA_VERSION,
        "banks": [{ "id": "factory", "name": "Factory", "order": 0 }],
        "presets": presets,
    })
}

pub fn runtime_descriptor(identity: &PackageIdentity) -> Value {
    json!({
        "schema_version": RUNTIME_SCHEMA_VERSION,
        "id": identity.id,
        "version": identity.version,
        "state_version": identity.state_version,
    })
}

/// Writes the three documents, or — with `check` — reports which of them the
/// working tree has let drift.
pub fn write(package: &Path, identity: &PackageIdentity, check: bool) -> Result<(), String> {
    let documents = [
        ("metadata/parameters.json", parameter_schema()),
        ("metadata/presets.json", preset_catalog()),
        ("metadata/runtime.json", runtime_descriptor(identity)),
    ];

    let mut stale = Vec::new();
    for (relative, value) in documents {
        let path = package.join(relative);
        let mut rendered =
            serde_json::to_string_pretty(&value).map_err(|error| format!("{relative}: {error}"))?;
        rendered.push('\n');
        if check {
            let current = std::fs::read_to_string(&path).unwrap_or_default();
            if current != rendered {
                stale.push(relative.to_owned());
            }
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        std::fs::write(&path, rendered).map_err(|error| format!("{}: {error}", path.display()))?;
        println!("wrote {}", path.display());
    }

    if !stale.is_empty() {
        return Err(format!(
            "generated metadata is out of date: {}. Run `cargo run -p rf-rig-lab -- metadata`.",
            stale.join(", ")
        ));
    }
    Ok(())
}
