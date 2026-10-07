//! ATIF v1.8, the normalized trajectory format (DECISIONS D14).
//!
//! Source: Harbor, "Agent Trajectory Format (ATIF)",
//! <https://www.harborframework.com/docs/agents/trajectory-format>, cross-checked against
//! <https://github.com/harbor-framework/harbor/blob/main/rfcs/0001-trajectory-format.md>.
//! Nothing outside `extra` deviates from that schema.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "ATIF-v1.8";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trajectory {
    pub schema_version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trajectory_id: Option<String>,
    pub agent: Agent,
    pub steps: Vec<Step>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub final_metrics: Option<FinalMetrics>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub continued_trajectory_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extra: Option<Extra>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub subagent_trajectories: Vec<Trajectory>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_definitions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extra: Option<Extra>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepSource {
    System,
    User,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub step_id: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timestamp: Option<String>,
    pub source: StepSource,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model_name: Option<String>,
    pub message: Message,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub observation: Option<Observation>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metrics: Option<Metrics>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub llm_call_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub is_copied_context: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extra: Option<Extra>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_call_id: String,
    pub function_name: String,
    pub arguments: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extra: Option<Extra>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub results: Vec<ObservationResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationResult {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content: Option<Message>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub subagent_trajectory_ref: Vec<SubagentRef>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extra: Option<Extra>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubagentRef {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trajectory_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trajectory_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub completion_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cached_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinalMetrics {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_completion_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_cached_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_steps: Option<u64>,
}

/// The /goal Cup extension. Everything tracker-specific lives under `extra.goalcup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extra {
    pub goalcup: GoalcupExtra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalcupExtra {
    /// Hash links back to the leg-1 records this step was built from.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub raw: Vec<RawLink>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawLink {
    pub store: String,
    pub seq: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pair_key: Option<String>,
}

impl Trajectory {
    pub fn new(agent: Agent) -> Trajectory {
        Trajectory {
            schema_version: SCHEMA_VERSION.to_string(),
            session_id: None,
            trajectory_id: None,
            agent,
            steps: Vec::new(),
            notes: None,
            final_metrics: None,
            continued_trajectory_ref: None,
            extra: None,
            subagent_trajectories: Vec::new(),
        }
    }

    /// Append a step, assigning the next `step_id` (ATIF numbers from 1).
    pub fn push(&mut self, mut step: Step) -> u64 {
        let id = self.steps.len() as u64 + 1;
        step.step_id = id;
        self.steps.push(step);
        id
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}
