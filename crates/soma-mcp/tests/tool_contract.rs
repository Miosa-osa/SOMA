use std::collections::BTreeMap;

use rmcp::ServiceExt as _;
use soma_mcp::{SomaMcpServer, UnavailableRuntime};

#[tokio::test]
async fn tool_catalog_has_bounded_schemas_and_accurate_hints() {
    let (server_transport, client_transport) = tokio::io::duplex(128 * 1024);
    let server_task = tokio::spawn(async move {
        SomaMcpServer::new(UnavailableRuntime)
            .serve(server_transport)
            .await
            .expect("server starts")
            .waiting()
            .await
    });
    let client = ().serve(client_transport).await.expect("client connects");

    let tools = client
        .list_tools(None)
        .await
        .expect("tool catalog")
        .tools
        .into_iter()
        .map(|tool| (tool.name.to_string(), tool))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        tools.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "soma_destroy",
            "soma_doctor",
            "soma_exec",
            "soma_file",
            "soma_inspect",
            "soma_launch",
            "soma_run",
            "soma_stop",
        ]
    );

    assert_annotations(&tools);
    assert_input_schemas(&tools);

    client.cancel().await.expect("client closes");
    server_task
        .await
        .expect("server task")
        .expect("server closes");
}

fn assert_annotations(tools: &BTreeMap<String, rmcp::model::Tool>) {
    for tool in tools.values() {
        let description = tool.description.as_deref().expect("tool description");
        assert!(description.contains("macOS is development-only"));
        assert!(description.contains("fail closed"));
    }
    for name in ["soma_doctor", "soma_inspect"] {
        let annotations = tools[name].annotations.as_ref().expect("annotations");
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.destructive_hint, Some(false));
        assert_eq!(annotations.idempotent_hint, Some(true));
    }
    for name in [
        "soma_run",
        "soma_launch",
        "soma_exec",
        "soma_file",
        "soma_stop",
        "soma_destroy",
    ] {
        let annotations = tools[name].annotations.as_ref().expect("annotations");
        assert_eq!(annotations.read_only_hint, Some(false));
        assert_eq!(annotations.idempotent_hint, Some(false));
    }
    for name in ["soma_stop", "soma_destroy"] {
        assert_eq!(
            tools[name]
                .annotations
                .as_ref()
                .expect("annotations")
                .destructive_hint,
            Some(true)
        );
    }
}

fn assert_input_schemas(tools: &BTreeMap<String, rmcp::model::Tool>) {
    for tool in tools.values() {
        assert_eq!(tool.input_schema["additionalProperties"], false);
        let properties = tool.input_schema["properties"]
            .as_object()
            .expect("properties");
        assert_eq!(properties["backend"]["default"], "auto");
        for forbidden in [
            "command",
            "env",
            "mounts",
            "runtime_path",
            "shell",
            "secret",
        ] {
            assert!(
                !properties.contains_key(forbidden),
                "{} exposes {forbidden}",
                tool.name
            );
        }
    }

    for name in ["soma_run", "soma_launch"] {
        let schema = &tools[name].input_schema;
        let properties = schema["properties"].as_object().expect("properties");
        for field in [
            "operation_id",
            "instance_id",
            "image",
            "display_name",
            "vcpu_count",
            "memory_mib",
            "storage_mib",
            "network",
        ] {
            assert!(properties.contains_key(field), "{name} lacks {field}");
        }
        assert_eq!(properties["vcpu_count"]["default"], 1);
        assert_eq!(properties["vcpu_count"]["minimum"], 1);
        assert_eq!(properties["vcpu_count"]["maximum"], u16::MAX);
        assert_eq!(properties["memory_mib"]["default"], 1024);
        assert_eq!(properties["memory_mib"]["minimum"], 1);
        assert!(properties["memory_mib"].get("maximum").is_none());
        assert_eq!(properties["storage_mib"]["default"], 10240);
        // Zero is a sandbox with no writable disk at all, which the schema must be able to ask
        // for; it is not a size below a floor.
        assert_eq!(properties["storage_mib"]["minimum"], 0);
        assert!(properties["storage_mib"].get("maximum").is_none());
        assert_eq!(properties["network"]["default"]["egress"], "denied");
        assert_eq!(properties["network"]["default"]["dns"], "denied");
        if name == "soma_run" {
            assert_eq!(properties["timeout_ms"]["default"], 30000);
            assert_eq!(properties["timeout_ms"]["minimum"], 1);
            assert_eq!(properties["timeout_ms"]["maximum"], 86_400_000);
            assert_eq!(properties["max_output_bytes"]["default"], 1_048_576);
            assert_eq!(properties["max_output_bytes"]["minimum"], 1);
            assert_eq!(properties["max_output_bytes"]["maximum"], 16_777_216);
        } else {
            assert!(!properties.contains_key("timeout_ms"));
            assert!(!properties.contains_key("max_output_bytes"));
        }
    }

    for name in ["soma_inspect", "soma_stop", "soma_destroy"] {
        assert!(
            !tools[name].input_schema["properties"]
                .as_object()
                .expect("properties")
                .contains_key("timeout_ms")
        );
    }
}
