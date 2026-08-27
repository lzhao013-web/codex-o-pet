use std::{collections::HashMap, time::Instant};

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeEvent {
    SessionStart {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    UserPromptSubmit {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
    },
    PreToolUse {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
    },
    PostToolUse {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
    },
    SubagentStart {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
    },
    SubagentStop {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
    },
    PreCompact {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
    },
    PostCompact {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
    },
    Stop {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
    },
}

impl BridgeEvent {
    fn session_id(&self) -> &str {
        match self {
            Self::SessionStart { session_id }
            | Self::UserPromptSubmit { session_id, .. }
            | Self::PreToolUse { session_id, .. }
            | Self::PostToolUse { session_id, .. }
            | Self::SubagentStart { session_id, .. }
            | Self::SubagentStop { session_id, .. }
            | Self::PreCompact { session_id, .. }
            | Self::PostCompact { session_id, .. }
            | Self::Stop { session_id, .. } => session_id,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct MappedBatch {
    pub session_id: String,
    pub events: Vec<Value>,
}

#[derive(Debug, Default)]
pub struct EventMapper {
    turn_started_at: HashMap<String, Instant>,
}

impl EventMapper {
    pub fn map(&mut self, event: BridgeEvent, now: Instant) -> MappedBatch {
        let session_id = event.session_id().to_owned();
        let events = match event {
            BridgeEvent::SessionStart { .. } => Vec::new(),
            BridgeEvent::UserPromptSubmit { turn_id, .. } => {
                let first_prompt = self.turn_started_at.insert(turn_id, now).is_none();
                vec![json!({
                    "type": if first_prompt { "agent_started" } else { "turn_started" }
                })]
            }
            BridgeEvent::PreToolUse {
                tool_name,
                tool_use_id,
                ..
            } => {
                if is_subagent_launcher(&tool_name) {
                    Vec::new()
                } else {
                    vec![json!({
                        "type": "tool_started",
                        "toolCallId": tool_use_id,
                        "toolName": normalize_tool_name(&tool_name),
                    })]
                }
            }
            BridgeEvent::PostToolUse {
                tool_name,
                tool_use_id,
                ..
            } => {
                if is_subagent_launcher(&tool_name) {
                    Vec::new()
                } else {
                    vec![json!({
                        "type": "tool_finished",
                        "toolCallId": tool_use_id,
                        "outcome": "success",
                    })]
                }
            }
            BridgeEvent::SubagentStart { agent_id, .. } => vec![json!({
                "type": "tool_started",
                "toolCallId": agent_id,
                "toolName": "skill",
            })],
            BridgeEvent::SubagentStop { agent_id, .. } => vec![json!({
                "type": "tool_finished",
                "toolCallId": agent_id,
                "outcome": "success",
            })],
            BridgeEvent::PreCompact { turn_id, .. } => vec![json!({
                "type": "tool_started",
                "toolCallId": compact_tool_call_id(&turn_id),
                "toolName": "skill",
            })],
            BridgeEvent::PostCompact { turn_id, .. } => vec![json!({
                "type": "tool_finished",
                "toolCallId": compact_tool_call_id(&turn_id),
                "outcome": "success",
            })],
            BridgeEvent::Stop { turn_id, .. } => {
                let duration_ms = self
                    .turn_started_at
                    .remove(&turn_id)
                    .map(|started| now.saturating_duration_since(started).as_millis())
                    .unwrap_or(0)
                    .try_into()
                    .unwrap_or(u64::MAX);
                vec![
                    json!({ "type": "reply_finished" }),
                    json!({
                        "type": "agent_settled",
                        "outcome": "success",
                        "durationMs": duration_ms,
                    }),
                ]
            }
        };

        MappedBatch { session_id, events }
    }
}

fn compact_tool_call_id(turn_id: &str) -> String {
    format!("compact:{turn_id}")
}

fn is_subagent_launcher(tool_name: &str) -> bool {
    tool_name.eq_ignore_ascii_case("spawn_agent") || tool_name.eq_ignore_ascii_case("agent")
}

fn normalize_tool_name(tool_name: &str) -> String {
    match tool_name.to_ascii_lowercase().as_str() {
        "bash" | "shell" | "commandexecution" => "bash".to_string(),
        "apply_patch" | "edit" | "write" | "filechange" => "write".to_string(),
        "read" => "read".to_string(),
        "grep" => "grep".to_string(),
        "glob" | "find" => "find".to_string(),
        "web_search" | "websearch" | "web_search_call" => "websearch".to_string(),
        "web_fetch" | "webfetch" => "webfetch".to_string(),
        "spawn_agent" | "agent" | "skill" => "skill".to_string(),
        _ => tool_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn maps_a_turn_and_successful_tool_lifecycle() {
        let mut mapper = EventMapper::default();
        let now = Instant::now();

        let started = mapper.map(
            BridgeEvent::UserPromptSubmit {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            },
            now,
        );
        assert_eq!(started.events, vec![json!({ "type": "agent_started" })]);

        let tool = mapper.map(
            BridgeEvent::PreToolUse {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                tool_name: "apply_patch".into(),
                tool_use_id: "tool-1".into(),
            },
            now,
        );
        assert_eq!(
            tool.events,
            vec![json!({
                "type": "tool_started",
                "toolCallId": "tool-1",
                "toolName": "write",
            })]
        );

        let finished = mapper.map(
            BridgeEvent::PostToolUse {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                tool_name: "apply_patch".into(),
                tool_use_id: "tool-1".into(),
            },
            now,
        );
        assert_eq!(
            finished.events,
            vec![json!({
                "type": "tool_finished",
                "toolCallId": "tool-1",
                "outcome": "success",
            })]
        );

        let stopped = mapper.map(
            BridgeEvent::Stop {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            },
            now + Duration::from_millis(1_234),
        );
        assert_eq!(stopped.events[0], json!({ "type": "reply_finished" }));
        assert_eq!(
            stopped.events[1],
            json!({
                "type": "agent_settled",
                "outcome": "success",
                "durationMs": 1_234,
            })
        );
    }

    #[test]
    fn treats_an_additional_prompt_in_the_same_turn_as_turn_activity() {
        let mut mapper = EventMapper::default();
        let now = Instant::now();
        let event = || BridgeEvent::UserPromptSubmit {
            session_id: "session-1".into(),
            turn_id: "turn-1".into(),
        };

        assert_eq!(
            mapper.map(event(), now).events,
            vec![json!({ "type": "agent_started" })]
        );
        assert_eq!(
            mapper.map(event(), now).events,
            vec![json!({ "type": "turn_started" })]
        );
    }

    #[test]
    fn keeps_unknown_tool_names_for_o_pet_tooling_fallback() {
        assert_eq!(
            normalize_tool_name("mcp__github__search"),
            "mcp__github__search"
        );
        assert_eq!(normalize_tool_name("Bash"), "bash");
        assert_eq!(normalize_tool_name("web_search"), "websearch");
    }

    #[test]
    fn maps_subagent_lifecycle_and_suppresses_the_launcher_tool() {
        let mut mapper = EventMapper::default();
        let now = Instant::now();

        let launcher = mapper.map(
            BridgeEvent::PreToolUse {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                tool_name: "spawn_agent".into(),
                tool_use_id: "tool-1".into(),
            },
            now,
        );
        assert!(launcher.events.is_empty());

        let started = mapper.map(
            BridgeEvent::SubagentStart {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                agent_id: "agent-1".into(),
            },
            now,
        );
        assert_eq!(
            started.events,
            vec![json!({
                "type": "tool_started",
                "toolCallId": "agent-1",
                "toolName": "skill",
            })]
        );

        let finished = mapper.map(
            BridgeEvent::SubagentStop {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                agent_id: "agent-1".into(),
            },
            now,
        );
        assert_eq!(
            finished.events,
            vec![json!({
                "type": "tool_finished",
                "toolCallId": "agent-1",
                "outcome": "success",
            })]
        );

        let launcher_finished = mapper.map(
            BridgeEvent::PostToolUse {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                tool_name: "SPAWN_AGENT".into(),
                tool_use_id: "tool-1".into(),
            },
            now,
        );
        assert!(launcher_finished.events.is_empty());

        let launcher_alias = mapper.map(
            BridgeEvent::PreToolUse {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                tool_name: "Agent".into(),
                tool_use_id: "tool-2".into(),
            },
            now,
        );
        assert!(launcher_alias.events.is_empty());
    }

    #[test]
    fn maps_compaction_as_one_skill_lifecycle() {
        let mut mapper = EventMapper::default();
        let now = Instant::now();

        let started = mapper.map(
            BridgeEvent::PreCompact {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            },
            now,
        );
        assert_eq!(
            started.events,
            vec![json!({
                "type": "tool_started",
                "toolCallId": "compact:turn-1",
                "toolName": "skill",
            })]
        );

        let finished = mapper.map(
            BridgeEvent::PostCompact {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            },
            now,
        );
        assert_eq!(
            finished.events,
            vec![json!({
                "type": "tool_finished",
                "toolCallId": "compact:turn-1",
                "outcome": "success",
            })]
        );
    }
}
