use std::{io, time::Instant};

use serde_json::{Value, json};

use crate::events::{BridgeEvent, EventMapper};

pub trait PetTransport {
    fn deliver(&mut self, session_id: &str, events: &[Value]) -> io::Result<()>;
}

pub struct McpServer<P> {
    pet: P,
    mapper: EventMapper,
    last_delivery_error: Option<String>,
}

impl<P: PetTransport> McpServer<P> {
    pub fn new(pet: P) -> Self {
        Self {
            pet,
            mapper: EventMapper::default(),
            last_delivery_error: None,
        }
    }

    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let message = match serde_json::from_str::<Value>(line) {
            Ok(Value::Object(message)) => message,
            Ok(_) => return Some(rpc_error(Value::Null, -32600, "request must be an object")),
            Err(error) => {
                return Some(rpc_error(
                    Value::Null,
                    -32700,
                    &format!("invalid JSON: {error}"),
                ));
            }
        };
        let id = message.get("id").cloned()?;
        let method = message.get("method").and_then(Value::as_str);
        let Some(method) = method else {
            return Some(rpc_error(id, -32600, "missing method"));
        };

        match method {
            "initialize" => {
                let protocol_version = message
                    .get("params")
                    .and_then(Value::as_object)
                    .and_then(|params| params.get("protocolVersion"))
                    .and_then(Value::as_str)
                    .unwrap_or("2025-06-18");
                Some(rpc_result(
                    id,
                    json!({
                        "protocolVersion": protocol_version,
                        "capabilities": { "tools": {} },
                        "serverInfo": {
                            "name": "codex-o-pet-bridge",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    }),
                ))
            }
            "ping" => Some(rpc_result(id, json!({}))),
            "tools/list" => Some(rpc_result(id, tools_list())),
            "tools/call" => Some(rpc_result(id, self.call_tool(&message))),
            _ => Some(rpc_error(id, -32601, "method not found")),
        }
    }

    fn call_tool(&mut self, message: &serde_json::Map<String, Value>) -> Value {
        let Some(params) = message.get("params").and_then(Value::as_object) else {
            return tool_error("missing tools/call params");
        };
        if params.get("name").and_then(Value::as_str) != Some("emit") {
            return tool_error("unknown tool");
        }
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let event = match serde_json::from_value::<BridgeEvent>(arguments) {
            Ok(event) => event,
            Err(error) => return tool_error(&format!("invalid emit arguments: {error}")),
        };
        let batch = self.mapper.map(event, Instant::now());
        match self.pet.deliver(&batch.session_id, &batch.events) {
            Ok(()) => {
                if self.last_delivery_error.take().is_some() {
                    eprintln!("codex-o-pet: reconnected to o-pet");
                }
            }
            Err(error) => {
                let error = error.to_string();
                if self.last_delivery_error.as_deref() != Some(error.as_str()) {
                    eprintln!("codex-o-pet: cannot deliver event to o-pet: {error}");
                }
                self.last_delivery_error = Some(error);
            }
        }

        json!({ "content": [], "isError": false })
    }
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        },
    })
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

fn tools_list() -> Value {
    json!({
        "tools": [{
            "name": "emit",
            "description": "Forward one trusted Codex lifecycle event to the local o-pet process.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": [
                            "session_start",
                            "user_prompt_submit",
                            "pre_tool_use",
                            "post_tool_use",
                            "subagent_start",
                            "subagent_stop",
                            "pre_compact",
                            "post_compact",
                            "stop"
                        ]
                    },
                    "sessionId": { "type": "string" },
                    "turnId": { "type": "string" },
                    "toolName": { "type": "string" },
                    "toolUseId": { "type": "string" },
                    "agentId": { "type": "string" }
                },
                "required": ["kind", "sessionId"],
                "additionalProperties": false
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": false
            },
            "_meta": {
                "ui": { "visibility": [] }
            }
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakePet {
        deliveries: Vec<(String, Vec<Value>)>,
    }

    impl PetTransport for FakePet {
        fn deliver(&mut self, session_id: &str, events: &[Value]) -> io::Result<()> {
            self.deliveries
                .push((session_id.to_string(), events.to_vec()));
            Ok(())
        }
    }

    #[test]
    fn initializes_with_the_client_protocol_version() {
        let mut server = McpServer::new(FakePet::default());
        let response = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
            )
            .expect("response");

        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(
            response["result"]["serverInfo"]["name"],
            "codex-o-pet-bridge"
        );
    }

    #[test]
    fn exposes_emit_as_a_model_hidden_tool() {
        let mut server = McpServer::new(FakePet::default());
        let response = server
            .handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
            .expect("response");
        let tool = &response["result"]["tools"][0];

        assert_eq!(tool["name"], "emit");
        assert_eq!(tool["_meta"]["ui"]["visibility"], json!([]));
    }

    #[test]
    fn forwards_a_valid_hook_event_without_returning_model_context() {
        let mut server = McpServer::new(FakePet::default());
        let response = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"emit","arguments":{"kind":"user_prompt_submit","sessionId":"session-1","turnId":"turn-1"}}}"#,
            )
            .expect("response");

        assert_eq!(
            response["result"],
            json!({ "content": [], "isError": false })
        );
        assert_eq!(server.pet.deliveries.len(), 1);
        assert_eq!(server.pet.deliveries[0].0, "session-1");
        assert_eq!(
            server.pet.deliveries[0].1,
            vec![json!({ "type": "agent_started" })]
        );
    }

    #[test]
    fn rejects_invalid_emit_arguments_at_the_tool_boundary() {
        let mut server = McpServer::new(FakePet::default());
        let response = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"emit","arguments":{"kind":"pre_tool_use","sessionId":"session-1"}}}"#,
            )
            .expect("response");

        assert_eq!(response["result"]["isError"], true);
        assert!(server.pet.deliveries.is_empty());
    }

    #[test]
    fn every_plugin_hook_input_matches_the_emit_contract() {
        let config = serde_json::from_str::<Value>(include_str!("../plugin/hooks/hooks.json"))
            .expect("valid plugin hooks JSON");
        let schema = tools_list();
        let schema_kinds = schema["tools"][0]["inputSchema"]["properties"]["kind"]["enum"]
            .as_array()
            .expect("emit kind enum");
        let hook_groups = config["hooks"].as_object().expect("hook event map");
        let mut input_count = 0;

        for groups in hook_groups.values() {
            for group in groups.as_array().expect("matcher groups") {
                for hook in group["hooks"].as_array().expect("hook handlers") {
                    let input = hook["input"].clone();
                    let kind = input["kind"].as_str().expect("hook input kind").to_string();
                    assert!(
                        schema_kinds
                            .iter()
                            .any(|candidate| candidate == kind.as_str()),
                        "hook kind {kind} is missing from the emit schema"
                    );
                    serde_json::from_value::<BridgeEvent>(input)
                        .unwrap_or_else(|error| panic!("hook kind {kind} is invalid: {error}"));
                    input_count += 1;
                }
            }
        }

        assert_eq!(input_count, 9);
    }
}
