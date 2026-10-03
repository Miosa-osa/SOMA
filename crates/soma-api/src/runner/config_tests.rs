use super::{RunnerConfig, control_plane_authority};

/// A complete configuration document; tests change the one field they are about.
pub(crate) fn document() -> serde_json::Value {
    serde_json::json!({
        "runner": "miosa-host-03",
        "host_tag": "3",
        "listen": "127.0.0.1:0",
        "public_domain": "run-us.miosa.ai",
        "tls": {"certificate": "/etc/soma-runner/tls/fullchain.pem", "private_key": "/etc/soma-runner/tls/privkey.pem"},
        "control_plane": {
            "url": "https://10.20.0.1:4443",
            "ca": "/etc/miosa/fleet/ca.pem",
            "certificate": "/etc/miosa/fleet/host.pem",
            "private_key": "/etc/miosa/fleet/host-key.pem"
        },
        "journal": {"directory": "/var/lib/soma-runner/journal"},
        "launch": {
            "image": "docker.io/library/node:22",
            "shape": serde_json::to_value(soma::MachineShape::new(1, 512, 2_048).expect("valid shape")).expect("encode"),
            "template_id": "miosa-sandbox-soma"
        }
    })
}

fn parse(document: &serde_json::Value) -> std::io::Result<RunnerConfig> {
    RunnerConfig::parse(&serde_json::to_vec(document).expect("encode"))
}

#[test]
fn a_complete_document_parses_with_defaults() {
    let config = parse(&document()).expect("the reference document parses");

    assert_eq!(config.host_tag, '3');
    assert_eq!(config.rate_per_second, 300);
    assert_eq!(config.feed_stale_after().as_secs(), 900);
    assert_eq!(config.launch.default_timeout_seconds, 300);
    assert_eq!(config.journal.batch_lines, 500);
    assert_eq!(
        config.admission(),
        1_024,
        "no per-thread cap unless configured"
    );
}

#[test]
fn rejects_a_tag_that_is_not_one_lowercase_hex_digit() {
    for tag in ["g", "A", "33"] {
        let mut document = document();
        document["host_tag"] = serde_json::json!(tag);
        assert!(parse(&document).is_err(), "tag {tag} must be refused");
    }
}

#[test]
fn rejects_an_unknown_field() {
    let mut document = document();
    document["listn"] = serde_json::json!("0.0.0.0:443");

    assert!(parse(&document).is_err());
}

#[test]
fn splits_the_control_plane_authority() {
    assert_eq!(
        control_plane_authority("https://compute-01.miosa.internal:4443").expect("parses"),
        ("compute-01.miosa.internal".to_owned(), 4443)
    );
    assert_eq!(
        control_plane_authority("https://10.20.0.1/").expect("parses"),
        ("10.20.0.1".to_owned(), 443)
    );
    assert!(control_plane_authority("http://10.20.0.1").is_err());
    assert!(control_plane_authority("https://10.20.0.1/feed").is_err());
}

#[test]
fn a_configured_admission_cap_wins() {
    let mut document = document();
    document["admission"] = serde_json::json!(64);

    assert_eq!(parse(&document).expect("parses").admission(), 64);
}
