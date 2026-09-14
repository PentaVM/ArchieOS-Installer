//! The protocol rules, against a toolbox whose tools only record that they ran.

use archie_mcp_server::{
    Clearance, Effect, McpServer, ToolOutcome, ToolSpec, Toolbox, MAX_RESULT_BYTES,
    SUPPORTED_VERSIONS,
};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};

#[derive(Default)]
struct Recorder {
    calls: Vec<(String, Map<String, Value>)>,
}

impl Toolbox for Recorder {
    fn tools(&self) -> Vec<ToolSpec> {
        let tool = |name: &'static str, grade: PrivacyGrade, effect: Effect| ToolSpec {
            name,
            title: name,
            description: "a test tool",
            input_schema: json!({"type": "object"}),
            grade,
            effect,
        };
        vec![
            tool("open_door", PrivacyGrade::Public, Effect::ReadOnly),
            tool("read_diary", PrivacyGrade::Internal, Effect::ReadOnly),
            tool("find_house", PrivacyGrade::Sensitive, Effect::ReadOnly),
            tool("fail", PrivacyGrade::Public, Effect::Acts),
            tool("flood", PrivacyGrade::Public, Effect::ReadOnly),
        ]
    }

    async fn call(&mut self, name: &str, arguments: Map<String, Value>) -> ToolOutcome {
        self.calls.push((name.to_string(), arguments));
        match name {
            "fail" => ToolOutcome::error("the door is locked"),
            "flood" => ToolOutcome::json(&json!("x".repeat(MAX_RESULT_BYTES + 1))),
            _ => ToolOutcome::json(&json!({"ran": name})),
        }
    }
}

async fn reply(server: &mut McpServer<Recorder>, message: Value) -> Value {
    serde_json::from_str(&server.handle(&message.to_string()).await.expect("a reply")).unwrap()
}

async fn initialized(clearance: Clearance) -> McpServer<Recorder> {
    let mut server = McpServer::new(Recorder::default(), clearance);
    let init = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}),
    )
    .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(server
        .handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string())
        .await
        .is_none());
    server
}

fn names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn initialize_agrees_on_a_version_the_server_speaks() {
    let mut server = McpServer::new(Recorder::default(), Clearance::Public);
    let old = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": "a", "method": "initialize", "params": {"protocolVersion": "2024-11-05"}}),
    )
    .await;
    assert_eq!(old["id"], "a", "string ids are echoed as strings");
    assert_eq!(old["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(
        old["result"]["capabilities"],
        json!({"tools": {"listChanged": false}})
    );
    assert_eq!(old["result"]["serverInfo"]["name"], "archie");

    let mut server = McpServer::new(Recorder::default(), Clearance::Public);
    let future = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "2099-01-01"}}),
    )
    .await;
    assert_eq!(
        future["result"]["protocolVersion"], SUPPORTED_VERSIONS[0],
        "an unknown version is answered with the newest this server speaks"
    );
}

#[tokio::test]
async fn nothing_but_ping_works_before_initialize() {
    let mut server = McpServer::new(Recorder::default(), Clearance::Internal);
    let pong = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
    )
    .await;
    assert_eq!(pong["result"], json!({}));
    let early = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    assert_eq!(early["error"]["code"], -32002);
    let call = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "open_door"}}),
    )
    .await;
    assert_eq!(call["error"]["code"], -32002);
}

#[tokio::test]
async fn a_tool_above_the_clearance_is_neither_listed_nor_callable() {
    let mut public = initialized(Clearance::Public).await;
    let list = reply(
        &mut public,
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/list"}),
    )
    .await;
    assert_eq!(names(&list), vec!["open_door", "fail", "flood"]);
    let hidden = reply(
        &mut public,
        json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "read_diary", "arguments": {}}}),
    )
    .await;
    let missing = reply(
        &mut public,
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "no_such_tool"}}),
    )
    .await;
    assert_eq!(
        hidden["error"], missing["error"],
        "a hidden tool and a missing one look the same"
    );
    assert_eq!(hidden["error"]["code"], -32602);

    let mut internal = initialized(Clearance::Internal).await;
    let list = reply(
        &mut internal,
        json!({"jsonrpc": "2.0", "id": 8, "method": "tools/list"}),
    )
    .await;
    assert_eq!(
        names(&list),
        vec!["open_door", "read_diary", "fail", "flood"]
    );
    let sensitive = reply(
        &mut internal,
        json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": "find_house"}}),
    )
    .await;
    assert_eq!(
        sensitive["error"]["code"], -32602,
        "internal is the ceiling"
    );

    assert_eq!(Clearance::parse("sensitive"), None);
    assert_eq!(Clearance::parse("secret"), None);
}

#[tokio::test]
async fn tool_results_and_tool_failures_are_results() {
    let mut server = initialized(Clearance::Public).await;
    let ok = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {"name": "open_door", "arguments": {"which": "front"}}}),
    )
    .await;
    assert_eq!(ok["result"]["isError"], false);
    assert_eq!(ok["result"]["content"][0]["type"], "text");
    let text: Value =
        serde_json::from_str(ok["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, json!({"ran": "open_door"}));

    let failed = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 11, "method": "tools/call", "params": {"name": "fail"}}),
    )
    .await;
    assert_eq!(failed["result"]["isError"], true);
    assert_eq!(failed["result"]["content"][0]["text"], "the door is locked");

    let flooded = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 12, "method": "tools/call", "params": {"name": "flood"}}),
    )
    .await;
    assert_eq!(
        flooded["result"]["isError"], true,
        "an oversized result is refused, never cut"
    );
    assert!(
        flooded["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .len()
            < 200
    );

    let list = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 13, "method": "tools/list"}),
    )
    .await;
    let fail = &list["result"]["tools"][1];
    assert_eq!(fail["annotations"]["readOnlyHint"], false);
    assert_eq!(fail["annotations"]["destructiveHint"], true);
}

#[tokio::test]
async fn malformed_calls_never_reach_a_tool() {
    let mut server = initialized(Clearance::Internal).await;
    for params in [
        json!("open_door"),
        json!({}),
        json!({"name": 3}),
        json!({"name": "open_door", "arguments": [1, 2]}),
    ] {
        let answer = reply(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 20, "method": "tools/call", "params": params}),
        )
        .await;
        assert_eq!(answer["error"]["code"], -32602, "{params}");
    }
    assert!(server.handle("{not json").await.unwrap().contains("-32700"));
    let batch: Value = serde_json::from_str(
        &server
            .handle(&json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}]).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(batch["error"]["code"], -32600, "batches are not accepted");
    let bad_id = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": {"x": 1}, "method": "ping"}),
    )
    .await;
    assert_eq!(bad_id["error"]["code"], -32600);
    assert_eq!(bad_id["id"], Value::Null);
    let wrong_version = reply(
        &mut server,
        json!({"jsonrpc": "1.0", "id": 4, "method": "ping"}),
    )
    .await;
    assert_eq!(wrong_version["error"]["code"], -32600);
    let unknown = reply(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 5, "method": "resources/list"}),
    )
    .await;
    assert_eq!(unknown["error"]["code"], -32601);
    assert!(
        !unknown.to_string().contains("resources/list"),
        "the method name the client sent is not echoed back"
    );

    // A response to a request the server never made, and a notification: no reply to either.
    assert!(server
        .handle(&json!({"jsonrpc": "2.0", "id": 99, "result": {}}).to_string())
        .await
        .is_none());
    assert!(server
        .handle(&json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}).to_string())
        .await
        .is_none());
}
