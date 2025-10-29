use std::io::{self, BufRead, Write};

use anyhow::{Context, Result};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::tool::ToolRegistry;

/// Run a minimal JSON-RPC 2.0 loop over stdio.
pub fn run_stdio_server(registry: &ToolRegistry) -> Result<()> {
    info!("Starting stdio JSON-RPC loop");
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) if !line.trim().is_empty() => line,
            Ok(_) => continue,
            Err(err) => {
                error!("Failed reading stdin: {err}");
                break;
            }
        };

        debug!("Received: {line}");
        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(err) => {
                let response =
                    JsonRpcResponse::error(None, JsonRpcError::parse_error(err.to_string()));
                write_response(&mut stdout, &response)?;
                continue;
            }
        };

        let response = handle_request(registry, request);
        write_response(&mut stdout, &response)?;
    }

    info!("Stdio loop terminated");
    Ok(())
}

fn handle_request(registry: &ToolRegistry, request: JsonRpcRequest) -> JsonRpcResponse {
    match request.method.as_str() {
        "ping" => JsonRpcResponse::result(request.id, json!({ "pong": true })),
        "tools.list" => {
            let descriptors = registry.descriptors();
            JsonRpcResponse::result(request.id, json!({ "tools": descriptors }))
        }
        "tools.call" => call_tool(registry, request),
        other => JsonRpcResponse::error(request.id, JsonRpcError::method_not_found(other)),
    }
}

fn call_tool(registry: &ToolRegistry, request: JsonRpcRequest) -> JsonRpcResponse {
    let id = request.id.clone();
    let params = match request.params {
        Some(Value::Object(map)) => map,
        _ => {
            return JsonRpcResponse::error(
                id,
                JsonRpcError::invalid_params("Expected object params"),
            );
        }
    };

    let tool_name = match params.get("tool") {
        Some(Value::String(name)) => name.clone(),
        _ => {
            return JsonRpcResponse::error(
                id,
                JsonRpcError::invalid_params("Missing `tool` string"),
            );
        }
    };
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

    match registry.call(&tool_name, arguments) {
        Ok(result) => JsonRpcResponse::result(id, json!({ "tool": tool_name, "result": result })),
        Err(err) => JsonRpcResponse::error(id, JsonRpcError::internal_error(err.to_string())),
    }
}

fn write_response(stdout: &mut impl Write, response: &JsonRpcResponse) -> Result<()> {
    let payload = serde_json::to_string(response).context("serialize response")?;
    debug!("Responding: {payload}");
    stdout
        .write_all(payload.as_bytes())
        .and_then(|_| stdout.write_all(b"\n"))
        .and_then(|_| stdout.flush())
        .context("write to stdout")
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default = "jsonrpc_tag")]
    _jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    id: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
}

impl JsonRpcResponse {
    fn result(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            result: Some(result),
            error: None,
            id,
        }
    }

    fn error(id: Option<Value>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(error),
            id,
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcError {
    fn parse_error(message: String) -> Self {
        Self::new(-32700, "Parse error", Some(json!({ "details": message })))
    }

    fn method_not_found(method: &str) -> Self {
        Self::new(
            -32601,
            "Method not found",
            Some(json!({ "method": method })),
        )
    }

    fn invalid_params(message: &str) -> Self {
        Self::new(
            -32602,
            "Invalid params",
            Some(json!({ "details": message })),
        )
    }

    fn internal_error(message: String) -> Self {
        Self::new(
            -32603,
            "Internal error",
            Some(json!({ "details": message })),
        )
    }

    fn new(code: i64, message: &str, data: Option<Value>) -> Self {
        Self {
            code,
            message: message.to_owned(),
            data,
        }
    }
}

fn jsonrpc_tag() -> String {
    "2.0".to_string()
}
