//! Public, serializable projection of the [`NodeTypeRegistry`] — the
//! builder palette's source of truth (task P3.1, PRD §6.3).
//!
//! PRD §6.3: *"Palette content derives from the registry (`config_schema`),
//! so `command` and future types appear automatically."* This module is
//! how that promise is kept: the registry is `pub(crate)` (handlers reach
//! deep into `ExecutionDriver`), so the editor can't read it directly.
//! [`node_type_catalog`] flattens each handler's self-description into a
//! plain data structure the `node_types_list` Tauri command hands to the
//! frontend verbatim.
//!
//! Because every field comes from the [`NodeHandler`] trait — and the
//! introducing fields ([`NodeHandler::display`]) have **no default** — a
//! new node type cannot be registered without also showing up in the
//! palette, fully labelled. That is the P3.5 `command`-node acceptance
//! test ("appears in the builder palette untouched") made structural
//! rather than aspirational.
//!
//! Aliases are deliberately excluded: `sequence`'s retired `"parallel"`
//! name still *resolves* for workflows cloned before the rename, but
//! offering it in the palette would mint new definitions on a dead name.

use serde::{Deserialize, Serialize};

use crate::domain::models::workflow_v2::PortType;

use super::registry::NodeTypeRegistry;

/// One entry in the builder palette: everything the editor needs to
/// offer a node type, validate a connection into or out of it, and (from
/// P3.2) render its config form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeTypeInfo {
    /// Registry key, and the value written to `NodeConfig.type`.
    pub kind: String,
    /// Palette title (`"Agent"`, `"Finalize"`).
    pub label: String,
    /// One-line description shown under the label.
    pub summary: String,
    /// JSON Schema for this type's `config` payload. The config side
    /// panel renders from this (P3.2).
    pub config_schema: serde_json::Value,
    /// Coarse port types this node accepts on incoming edges.
    pub inputs: Vec<PortType>,
    /// Coarse port types it produces. Empty means "sink" — nothing may
    /// connect out of it.
    pub outputs: Vec<PortType>,
    /// Cap on how many of this type one workflow may hold; `None` is
    /// unbounded. The palette greys the entry out at the cap.
    pub max_instances: Option<u32>,
}

/// Every registered node type, in registration order.
pub fn node_type_catalog() -> Vec<NodeTypeInfo> {
    NodeTypeRegistry::global()
        .handlers()
        .iter()
        .map(|h| {
            let display = h.display();
            let ports = h.ports();
            NodeTypeInfo {
                kind: h.kind().to_string(),
                label: display.label.to_string(),
                summary: display.summary.to_string(),
                config_schema: h.config_schema().clone(),
                inputs: ports.inputs.to_vec(),
                outputs: ports.outputs.to_vec(),
                max_instances: h.max_instances(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_every_registered_kind_in_order() {
        let catalog = node_type_catalog();
        let kinds: Vec<&str> = catalog.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, NodeTypeRegistry::global().kinds());
    }

    #[test]
    fn every_entry_introduces_itself() {
        // The palette is only self-maintaining if a registered type cannot
        // reach it half-described — this is the assertion that keeps
        // P3.5's "zero frontend edits" honest.
        for entry in node_type_catalog() {
            assert!(
                !entry.label.trim().is_empty(),
                "{} has no label",
                entry.kind
            );
            assert!(
                !entry.summary.trim().is_empty(),
                "{} has no summary",
                entry.kind
            );
            assert!(
                entry.config_schema.is_object(),
                "{} config schema is not an object",
                entry.kind
            );
            assert!(
                !entry.inputs.is_empty(),
                "{} accepts nothing — it could never be connected",
                entry.kind
            );
        }
    }

    #[test]
    fn retired_aliases_are_not_offered() {
        // `parallel` still resolves in `handler_for`, but minting new
        // definitions on a dead kind name is exactly what the palette
        // must not invite.
        assert!(node_type_catalog().iter().all(|e| e.kind != "parallel"));
    }

    #[test]
    fn finalize_is_a_capped_sink() {
        let finalize = node_type_catalog()
            .into_iter()
            .find(|e| e.kind == "finalize")
            .expect("finalize is registered");
        // No outputs is what makes the editor refuse an edge out of it
        // (the `finalize-not-sink` lint rule, enforced at connect time).
        assert!(finalize.outputs.is_empty());
        assert_eq!(finalize.max_instances, Some(1));
    }

    /// The config side panel (P3.2) renders a control per schema property,
    /// choosing it from the property's `type`/`enum`. A property that declares
    /// neither would fall through to the panel's raw-JSON escape hatch — safe,
    /// but a silent downgrade nobody would notice. Assert the schemas stay
    /// within the vocabulary the renderer actually models.
    #[test]
    fn every_config_property_is_renderable() {
        const MODELLED: [&str; 6] = ["string", "boolean", "integer", "number", "object", "array"];
        for entry in node_type_catalog() {
            let Some(props) = entry
                .config_schema
                .get("properties")
                .and_then(|p| p.as_object())
            else {
                continue; // a type with no config at all is fine
            };
            for (key, spec) in props {
                if spec.get("enum").is_some() {
                    continue;
                }
                let ty = spec.get("type").unwrap_or_else(|| {
                    panic!("{}.{key} declares neither `type` nor `enum`", entry.kind)
                });
                let names: Vec<&str> = match ty {
                    serde_json::Value::String(s) => vec![s.as_str()],
                    serde_json::Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
                    other => panic!("{}.{key} has a non-schema `type`: {other}", entry.kind),
                };
                assert!(
                    names.iter().any(|n| MODELLED.contains(n)),
                    "{}.{key} declares only unmodelled types {names:?}",
                    entry.kind
                );
            }
        }
    }

    /// Companion to `canvas_fixtures_are_current`: the config panel's tests
    /// render from the **real** registry schemas rather than hand-written
    /// stand-ins, so a schema change that breaks the form is caught in Rust
    /// (fixture goes stale) instead of shipping a panel that silently stops
    /// offering a field. Regenerate with
    /// `UPDATE_CANVAS_FIXTURES=1 cargo test -p demeteo-core catalog_fixture`.
    #[test]
    fn catalog_fixture_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/components/canvas/__fixtures__/node_catalog.json");
        let json = serde_json::to_string_pretty(&node_type_catalog()).expect("serialize") + "\n";
        if std::env::var("UPDATE_CANVAS_FIXTURES").is_ok() {
            std::fs::write(&path, &json).expect("write catalog fixture");
            return;
        }
        let existing = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "catalog fixture {} missing: {e}; run UPDATE_CANVAS_FIXTURES=1",
                path.display()
            )
        });
        assert_eq!(
            existing, json,
            "node_catalog.json is stale; run UPDATE_CANVAS_FIXTURES=1"
        );
    }

    #[test]
    fn the_shipped_starter_shapes_stay_connectable() {
        // Guard against a future handler narrowing its inputs into
        // rejecting a graph the engine actually runs: every starter wires
        // agent→agent, agent→gate, gate→sequence, sequence→agent and
        // agent→finalize, so each of those must pass the port check.
        let by_kind: std::collections::HashMap<String, NodeTypeInfo> = node_type_catalog()
            .into_iter()
            .map(|e| (e.kind.clone(), e))
            .collect();
        for (from, to) in [
            ("agent", "agent"),
            ("agent", "gate"),
            ("gate", "sequence"),
            ("sequence", "agent"),
            ("agent", "finalize"),
            ("sync", "agent"),
        ] {
            let out = &by_kind[from].outputs;
            let inc = &by_kind[to].inputs;
            assert!(
                out.iter()
                    .any(|o| inc.iter().any(|i| o.compatible_with(*i))),
                "{from} → {to} must stay connectable"
            );
        }
    }
}
