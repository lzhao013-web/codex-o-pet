use std::{collections::HashMap, time::Instant};

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
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
        #[serde(rename = "agentType")]
        agent_type: String,
    },
    SubagentStop {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "agentType")]
        agent_type: String,
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

    pub fn diagnostic_summary(&self) -> String {
        match self {
            Self::PreToolUse { tool_name, .. } => {
                format!("hook=pre_tool_use tool={tool_name:?}")
            }
            Self::PostToolUse { tool_name, .. } => {
                format!("hook=post_tool_use tool={tool_name:?}")
            }
            Self::SubagentStart { agent_type, .. } => {
                format!("hook=subagent_start agent_type={agent_type:?}")
            }
            Self::SubagentStop { agent_type, .. } => {
                format!("hook=subagent_stop agent_type={agent_type:?}")
            }
            Self::SessionStart { .. } => "hook=session_start".to_string(),
            Self::UserPromptSubmit { .. } => "hook=user_prompt_submit".to_string(),
            Self::PreCompact { .. } => "hook=pre_compact".to_string(),
            Self::PostCompact { .. } => "hook=post_compact".to_string(),
            Self::Stop { .. } => "hook=stop".to_string(),
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
    turn_started_at: HashMap<(String, String), Instant>,
}

impl EventMapper {
    pub fn map(&mut self, event: BridgeEvent, now: Instant) -> MappedBatch {
        let session_id = event.session_id().to_owned();
        let events = match event {
            BridgeEvent::SessionStart { .. } => Vec::new(),
            BridgeEvent::UserPromptSubmit { turn_id, .. } => {
                let first_prompt = self
                    .turn_started_at
                    .insert((session_id.clone(), turn_id), now)
                    .is_none();
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
            BridgeEvent::SubagentStart {
                agent_id,
                agent_type,
                ..
            } => vec![json!({
                "type": "tool_started",
                "toolCallId": agent_id,
                "toolName": normalize_agent_type(&agent_type),
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
                    .remove(&(session_id.clone(), turn_id))
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
    let lowercase = tool_name.to_ascii_lowercase();
    match lowercase.as_str() {
        "bash" | "shell" | "commandexecution" => "bash".to_string(),
        "apply_patch" | "edit" | "write" | "filechange" => "write".to_string(),
        "read" => "read".to_string(),
        "grep" => "grep".to_string(),
        "glob" | "find" => "find".to_string(),
        "web_search" | "websearch" | "web_search_call" => "websearch".to_string(),
        "web_fetch" | "webfetch" => "webfetch".to_string(),
        "spawn_agent" | "agent" | "skill" => "skill".to_string(),
        _ => semantic_tool_name(&lowercase)
            .map(str::to_string)
            .unwrap_or_else(|| tool_name.to_string()),
    }
}

fn semantic_tool_name(tool_name: &str) -> Option<&'static str> {
    let tokens = tool_name
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let has = |candidates: &[&str]| {
        tokens
            .iter()
            .any(|token| candidates.iter().any(|candidate| token == candidate))
    };

    if has(&["agent", "delegate", "goal", "plan", "skill"]) {
        Some("skill")
    } else if has(&["bash", "command", "exec", "shell", "terminal"]) {
        Some("bash")
    } else if has(&["download", "fetch", "receive"]) {
        Some("webfetch")
    } else if has(&[
        "apply", "copy", "create", "delete", "edit", "move", "patch", "remove", "replace",
        "update", "write",
    ]) {
        Some("write")
    } else if has(&["browse", "lookup", "search"]) {
        Some("websearch")
    } else if has(&[
        "find", "get", "grep", "inspect", "list", "open", "query", "read", "view",
    ]) {
        Some("read")
    } else {
        None
    }
}

fn normalize_agent_type(agent_type: &str) -> String {
    match agent_type.to_ascii_lowercase().as_str() {
        "bug-analyzer" | "code-reviewer" | "explorer" => "read".to_string(),
        "ui-sketcher" | "worker" => "write".to_string(),
        _ => "skill".to_string(),
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
    fn classifies_semantic_tool_names_without_reading_tool_input() {
        assert_eq!(normalize_tool_name("mcp__github__search"), "websearch");
        assert_eq!(normalize_tool_name("mcp__filesystem__read_file"), "read");
        assert_eq!(normalize_tool_name("mcp__github__create_issue"), "write");
        assert_eq!(normalize_tool_name("update_plan"), "skill");
        assert_eq!(normalize_tool_name("Bash"), "bash");
        assert_eq!(normalize_tool_name("web_search"), "websearch");
        assert_eq!(normalize_tool_name("custom_tool"), "custom_tool");
    }

    #[test]
    fn tracks_equal_turn_ids_independently_across_sessions() {
        let mut mapper = EventMapper::default();
        let now = Instant::now();

        for session_id in ["session-1", "session-2"] {
            let started = mapper.map(
                BridgeEvent::UserPromptSubmit {
                    session_id: session_id.into(),
                    turn_id: "turn-1".into(),
                },
                now,
            );
            assert_eq!(started.events, vec![json!({ "type": "agent_started" })]);
        }

        let stopped = mapper.map(
            BridgeEvent::Stop {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            },
            now + Duration::from_millis(250),
        );
        assert_eq!(stopped.events[1]["durationMs"], 250);

        let continued = mapper.map(
            BridgeEvent::UserPromptSubmit {
                session_id: "session-2".into(),
                turn_id: "turn-1".into(),
            },
            now + Duration::from_millis(500),
        );
        assert_eq!(continued.events, vec![json!({ "type": "turn_started" })]);
    }

    #[test]
    fn rejects_unknown_bridge_event_fields() {
        let event = json!({
            "kind": "user_prompt_submit",
            "sessionId": "session-1",
            "turnId": "turn-1",
            "prompt": "must not cross the bridge boundary",
        });

        assert!(serde_json::from_value::<BridgeEvent>(event).is_err());
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
                agent_type: "explorer".into(),
            },
            now,
        );
        assert_eq!(
            started.events,
            vec![json!({
                "type": "tool_started",
                "toolCallId": "agent-1",
                "toolName": "read",
            })]
        );

        let finished = mapper.map(
            BridgeEvent::SubagentStop {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                agent_id: "agent-1".into(),
                agent_type: "explorer".into(),
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
    fn maps_known_subagent_roles_to_specific_activities() {
        assert_eq!(normalize_agent_type("explorer"), "read");
        assert_eq!(normalize_agent_type("code-reviewer"), "read");
        assert_eq!(normalize_agent_type("bug-analyzer"), "read");
        assert_eq!(normalize_agent_type("worker"), "write");
        assert_eq!(normalize_agent_type("ui-sketcher"), "write");
        assert_eq!(normalize_agent_type("custom-agent"), "skill");
    }

    #[test]
    fn diagnostics_include_only_lifecycle_metadata() {
        let event = BridgeEvent::PreToolUse {
            session_id: "private-session-id".into(),
            turn_id: "private-turn-id".into(),
            tool_name: "mcp__github__search".into(),
            tool_use_id: "private-tool-id".into(),
        };

        assert_eq!(
            event.diagnostic_summary(),
            r#"hook=pre_tool_use tool="mcp__github__search""#
        );
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
