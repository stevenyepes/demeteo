// Tests extracted from `src/adapters/mcp/mcp_handler.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::mcp_handler`.
//
// implementation-spec.md §5's drift guard: `docs/mcp-skill/SKILL.md`'s
// routing table must name every tool `tool_catalog()` serves. Reads the
// catalog's live name list rather than a hand-copied literal, so an added,
// renamed, or removed tool fails this test instead of rotting the skill
// silently.

use super::*;

const SKILL_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/mcp-skill/SKILL.md"
));

fn catalog_tool_names() -> Vec<String> {
    tool_catalog()
        .as_array()
        .expect("tool_catalog() returns a JSON array")
        .iter()
        .map(|tool| {
            tool["name"]
                .as_str()
                .expect("each tool descriptor has a string \"name\" field")
                .to_string()
        })
        .collect()
}

fn routing_table_section() -> &'static str {
    const START: &str = "## Tool routing table";
    const END: &str = "## Spend vs. read";
    let start = SKILL_MD
        .find(START)
        .expect("SKILL.md must have a \"## Tool routing table\" heading")
        + START.len();
    let end = SKILL_MD[start..]
        .find(END)
        .map(|offset| start + offset)
        .expect("SKILL.md must have a \"## Spend vs. read\" heading after the routing table");
    &SKILL_MD[start..end]
}

#[test]
fn every_catalog_tool_name_appears_in_the_skill() {
    let names = catalog_tool_names();

    assert_eq!(
        names.len(),
        12,
        "expected exactly twelve tools in tool_catalog(), found {}: {names:?}",
        names.len()
    );

    let routing_table = routing_table_section();
    for name in &names {
        let row = format!("| `{name}` |");
        assert!(
            routing_table.contains(row.as_str()),
            "tool {name:?} from tool_catalog() is missing a row in the \
             \"## Tool routing table\" section of docs/mcp-skill/SKILL.md \
             (mentioning it elsewhere in the file, e.g. in passing prose, \
             does not count)"
        );
    }
}

/// Critic review Major #1: the "Spend vs. read" section once claimed every
/// tool but `start_feature`/`start_ticket` "changes nothing," which is false
/// for `create_workspace_project` and `apply_run_shape_patch` — both are free
/// of charge but persist a real write. Guards against that overclaim
/// regressing.
#[test]
fn spend_vs_read_does_not_overclaim_free_reads() {
    const READ_ONLY_TOOLS: [&str; 8] = [
        "list_projects",
        "list_features",
        "get_feature",
        "list_step_attempts",
        "get_failure_verdict",
        "list_pending_gates",
        "get_discovery_board",
        "run_events_since",
    ];

    let names = catalog_tool_names();
    for name in READ_ONLY_TOOLS {
        assert!(
            names.iter().any(|n| n == name),
            "read-only tool {name:?} asserted by this test is missing from tool_catalog()"
        );
    }
    assert_eq!(
        names.len(),
        READ_ONLY_TOOLS.len() + 4,
        "expected the read-only set plus create_workspace_project, \
         apply_run_shape_patch, start_feature, and start_ticket to cover the \
         full catalog, found {}: {names:?}",
        names.len()
    );

    assert!(
        !SKILL_MD.contains("is a free read: it inspects state and changes nothing"),
        "SKILL.md's \"Spend vs. read\" section must not claim every non-spend \
         tool changes nothing — create_workspace_project and \
         apply_run_shape_patch are free of charge but persist real writes"
    );

    assert!(
        SKILL_MD.contains("create_workspace_project") && SKILL_MD.contains("apply_run_shape_patch"),
        "SKILL.md must name both create_workspace_project and \
         apply_run_shape_patch when distinguishing cost from side effects"
    );
    assert!(
        SKILL_MD.contains("real, persisted write"),
        "SKILL.md must distinguish \"free of charge\" from \"changes nothing\" \
         with explicit side-effect language (expected the phrase \"real, \
         persisted write\")"
    );
}
