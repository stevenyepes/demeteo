//! NodeHandler registration (P1.7).

use crate::adapters::step_executor::steps::StepOutcome;

use super::schema::SEQUENCE_CONFIG_SCHEMA;

/// The `sequence` node type behind the [`NodeHandler`] seam. Pure
/// delegation: execution is [`ExecutionDriver::handle_sequence_step`],
/// byte-for-byte the behavior the old `match` arm dispatched. Owns the
/// retired `parallel` alias (see the [module docs](super)) so workflows
/// the user cloned before the rename keep running.
///
/// [`NodeHandler`]: crate::adapters::step_executor::registry::NodeHandler
/// [`ExecutionDriver::handle_sequence_step`]: crate::adapters::step_executor::driver::ExecutionDriver::handle_sequence_step
pub(crate) struct SequenceNodeHandler;

#[async_trait::async_trait]
impl crate::adapters::step_executor::registry::NodeHandler for SequenceNodeHandler {
    fn kind(&self) -> &'static str {
        "sequence"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // The superseded name. Its concurrent fan-out was removed; such
        // steps now run their tasks sequentially. Kept so workflows the
        // user cloned or overrode keep running instead of failing with
        // "Unknown step kind".
        &["parallel"]
    }

    fn config_schema(&self) -> &'static serde_json::Value {
        &SEQUENCE_CONFIG_SCHEMA
    }

    fn display(&self) -> crate::adapters::step_executor::registry::NodeDisplay {
        crate::adapters::step_executor::registry::NodeDisplay {
            label: "Sequence",
            summary: "Fan a task list into one agent turn per task, \
                      checkpointing each task as it lands.",
        }
    }

    fn ports(&self) -> crate::adapters::step_executor::registry::NodePorts {
        use crate::domain::models::workflow_v2::PortType;
        crate::adapters::step_executor::registry::NodePorts {
            // A sequence *consumes* a task list, but not necessarily from
            // every predecessor: the shipped starters also wire a gate
            // straight into one. Input stays `Any`; in v2 the task-list
            // binding is the incoming edge, not a config field.
            inputs: &[PortType::Any],
            outputs: &[PortType::Text, PortType::File],
        }
    }

    /// Catch a **stray `task_list_from` in the config** before it costs a run.
    ///
    /// v2 expresses the binding as an edge — `migrate_v1_to_v2` lifts the
    /// field out and `project_v2_to_v1` recovers it from the graph — so the
    /// builder can never produce one. A hand-edited or imported v2 document
    /// can: the schema allows additional properties, and `project_node` only
    /// overwrites the key when the graph actually has a task-list edge to
    /// derive it from. Left alone, a bad value surfaces as a mid-run
    /// `NonRetryable` from `load_task_list_artifact`, which is the latest
    /// possible moment to learn about it.
    fn lint(
        &self,
        node: &crate::domain::models::workflow_v2::NodeConfig,
        graph: &crate::domain::workflow_graph::WorkflowGraph,
    ) -> Vec<crate::domain::workflow_graph::LintFinding> {
        use crate::domain::ids::StepId;
        use crate::domain::workflow_graph::LintFinding;

        let Some(source) = node
            .config
            .get("task_list_from")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Vec::new();
        };

        if !graph.contains(&StepId::from(source)) {
            return vec![LintFinding::node_error(
                "task-list-unknown-source",
                &node.id,
                format!(
                    "sequence node '{}' reads its task list from '{source}', which this \
                     workflow does not contain",
                    node.id
                ),
            )];
        }
        vec![LintFinding::node_warning(
            "task-list-legacy-binding",
            &node.id,
            format!(
                "sequence node '{}' carries a v1 `task_list_from` pointing at '{source}'. \
                 Schema v2 expresses that as an edge — draw one from '{source}' and delete \
                 the field, or it acts as a dependency the canvas never shows.",
                node.id
            ),
        )]
    }

    async fn execute(
        &self,
        ctx: crate::adapters::step_executor::registry::NodeCtx<'_>,
    ) -> StepOutcome {
        ctx.driver
            .handle_sequence_step(
                ctx.step_exec,
                ctx.step_conf,
                ctx.accumulated_cost,
                ctx.accumulated_tokens,
                ctx.step_start,
                ctx.step_index,
                ctx.step_execs,
            )
            .await
    }
}
