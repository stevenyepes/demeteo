//! NodeHandler registration (P1.6).

use crate::adapters::step_executor::steps::StepOutcome;

use super::context::{AgentSpend, AgentStepCtx};
use super::schema::AGENT_CONFIG_SCHEMA;

/// The `agent` node type behind the [`NodeHandler`] seam. Pure
/// delegation: execution is [`ExecutionDriver::handle_agent_step`],
/// byte-for-byte the behavior the old `match` arm dispatched.
///
/// [`NodeHandler`]: crate::adapters::step_executor::registry::NodeHandler
pub(crate) struct AgentNodeHandler;

#[async_trait::async_trait]
impl crate::adapters::step_executor::registry::NodeHandler for AgentNodeHandler {
    fn kind(&self) -> &'static str {
        "agent"
    }

    fn config_schema(&self) -> &'static serde_json::Value {
        &AGENT_CONFIG_SCHEMA
    }

    fn display(&self) -> crate::adapters::step_executor::registry::NodeDisplay {
        crate::adapters::step_executor::registry::NodeDisplay {
            label: "Agent",
            summary: "One agent turn against the feature worktree: writes the \
                      declared artifacts, optionally checked by a verifier.",
        }
    }

    fn ports(&self) -> crate::adapters::step_executor::registry::NodePorts {
        use crate::domain::models::workflow_v2::PortType;
        crate::adapters::step_executor::registry::NodePorts {
            inputs: &[PortType::Any],
            // An agent turn can emit prose, files, a task plan (the v1
            // `task-list.json` a sequence node consumes), and — when a
            // verifier is attached — a verdict.
            outputs: &[
                PortType::Text,
                PortType::File,
                PortType::TaskList,
                PortType::Verdict,
            ],
        }
    }

    /// A `review_artifact` that marks no review says so at save time.
    ///
    /// Nothing else does: the step runs as an ordinary agent step, without the
    /// review format, and the only symptom is a ship gate that reports no
    /// review step covers it — on a run, at the far end of the workflow from
    /// the field that is wrong.
    fn lint(
        &self,
        node: &crate::domain::models::workflow_v2::NodeConfig,
        _graph: &crate::domain::workflow_graph::WorkflowGraph,
    ) -> Vec<crate::domain::workflow_graph::LintFinding> {
        use crate::domain::artifact::ArtifactDecl;
        use crate::domain::gate::review_contract::declared_review_path;
        use crate::domain::workflow_graph::LintFinding;

        let Some(name) = node.config.get("review_artifact").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let artifacts: Vec<ArtifactDecl> = node
            .config
            .get("artifacts")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        if declared_review_path(name, &artifacts).is_some() {
            return Vec::new();
        }
        vec![LintFinding::node_warning(
            "review-artifact-unreadable",
            &node.id,
            format!(
                "agent node '{}' names '{name}' as its review artifact, but declares no \
                 artifact of that name written to a path. It will run as an ordinary step: \
                 no review format is added to its prompt, and a ship gate after it is \
                 never approved on its review.",
                node.id
            ),
        )]
    }

    async fn execute(
        &self,
        ctx: crate::adapters::step_executor::registry::NodeCtx<'_>,
    ) -> StepOutcome {
        ctx.driver
            .handle_agent_step(
                AgentStepCtx {
                    step_exec: ctx.step_exec,
                    step_conf: ctx.step_conf,
                    step_index: ctx.step_index,
                    step_execs: ctx.step_execs,
                },
                AgentSpend {
                    cost: ctx.accumulated_cost,
                    tokens: ctx.accumulated_tokens,
                    start: ctx.step_start,
                    cache_read: ctx.out_cache_read,
                    cache_creation: ctx.out_cache_creation,
                },
            )
            .await
    }
}
