use super::{
    Created, Destroyed, Executed, ExecutedData, PlatformError, create_unavailable, encode, health,
    misdirected, refusal,
};

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("UTF-8")
}

#[test]
fn the_compact_create_body_is_the_fast_lane_bytes_plus_runner_url_and_create_ms() {
    let body = encode(&Created {
        cpu_count: 1,
        create_ms: 42,
        created_at: "2026-10-02T17:04:05.123456Z",
        deletion_pending: false,
        id: "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
        memory_mb: 512,
        name: None,
        runner_url: "https://3.run-us.miosa.ai",
        slug: "3f2504e0",
        state: "running",
        template_id: "miosa-sandbox-soma",
        timeout_sec: 3600,
    });

    assert_eq!(
        text(&body),
        r#"{"cpu_count":1,"create_ms":42,"created_at":"2026-10-02T17:04:05.123456Z","deletion_pending":false,"id":"3f2504e0-4f89-41d3-9a0c-0305e82c3301","memory_mb":512,"name":null,"runner_url":"https://3.run-us.miosa.ai","slug":"3f2504e0","state":"running","template_id":"miosa-sandbox-soma","timeout_sec":3600}"#
    );
}

#[test]
fn the_exec_and_destroy_bodies_match_the_fast_lane_bytes() {
    let exec = encode(&Executed {
        data: ExecutedData {
            exit_code: 0,
            stderr: "",
            stdout: "v22.23.2\n",
        },
    });
    assert_eq!(
        text(&exec),
        r#"{"data":{"exit_code":0,"stderr":"","stdout":"v22.23.2\n"}}"#
    );

    let destroyed = encode(&Destroyed {
        cpu_ms: None,
        lifetime_ms: 1234,
        id: "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
        mem_peak_bytes: None,
        operation_id: None,
        state: "destroyed",
        total_runtime_sec: None,
    });
    assert_eq!(
        text(&destroyed),
        r#"{"cpu_ms":null,"id":"3f2504e0-4f89-41d3-9a0c-0305e82c3301","lifetime_ms":1234,"mem_peak_bytes":null,"operation_id":null,"state":"destroyed","total_runtime_sec":null}"#
    );
}

#[test]
fn platform_errors_render_like_web_api_error() {
    assert_eq!(
        text(&PlatformError::sandbox_not_found().body()),
        r#"{"error":{"code":"NOT_FOUND","message":"sandbox not found","retryable":false},"ok":false}"#
    );
    assert_eq!(
        text(&PlatformError::missing_command().body()),
        r#"{"error":{"code":"MISSING_PARAM","details":{"field":"command"},"message":"missing required parameter: command","retryable":false},"ok":false}"#
    );
    assert_eq!(
        text(&PlatformError::destroy_busy().body()),
        r#"{"error":{"code":"SANDBOX_BUSY","message":"Another lifecycle operation is in progress for this sandbox","retry_after_ms":200,"retryable":true},"ok":false}"#
    );
    assert_eq!(
        text(&create_unavailable()),
        r#"{"error":{"code":"SOMA_FAST_LANE_UNAVAILABLE","message":"The SOMA fast lane could not claim a prepared sandbox right now. Retry in a moment.","retry_after":1,"retryable":true}}"#
    );
}

#[test]
fn runner_refusals_match_contract_c2() {
    assert_eq!(
        text(&refusal("unauthorized")),
        r#"{"error":"unauthorized"}"#
    );
    assert_eq!(
        text(&misdirected("https://a.run-us.miosa.ai")),
        r#"{"error":"misdirected","runner_url":"https://a.run-us.miosa.ai"}"#
    );
    assert_eq!(
        text(&health('3', 1200, Some(18))),
        r#"{"ok":true,"tag":"3","feed_age_ms":1200,"pool_ready":18}"#
    );
}
