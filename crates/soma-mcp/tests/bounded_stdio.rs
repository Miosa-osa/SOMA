use soma_mcp::MAX_INBOUND_MESSAGE_BYTES;
use std::{
    io::Write as _,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn oversized_unterminated_input_closes_without_protocol_output() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_soma-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start soma-mcp");
    let mut stdin = child.stdin.take().expect("child stdin");
    stdin
        .write_all(&vec![b' '; MAX_INBOUND_MESSAGE_BYTES + 1])
        .expect("write oversized input");

    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll soma-mcp") {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().expect("terminate unbounded server");
            panic!("soma-mcp did not reject input at the byte bound");
        }
        thread::sleep(Duration::from_millis(20));
    };

    assert!(!status.success(), "protocol abuse must fail closed");
    let output = child.wait_with_output().expect("collect output");
    assert!(
        output.stdout.is_empty(),
        "stdout must remain MCP protocol only"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("inbound MCP message exceeded"));
}
