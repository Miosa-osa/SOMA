use std::net::{IpAddr, Ipv4Addr};

use serde_json::{Value, json};

use crate::{NetworkAttachment, PublishedPort, TransportProtocol};

use super::super::fixtures::{INSTANCE, backend, control_limits, instance, success};

fn inspection_document(
    configured_networks: &Value,
    active_networks: &Value,
    nameservers: &Value,
    published_ports: &Value,
) -> Vec<u8> {
    serde_json::to_vec(&json!([{
        "configuration": {
            "id": format!("soma-{INSTANCE}"),
            "labels": {"io.miosa.soma.instance": INSTANCE},
            "networks": configured_networks,
            "dns": {"nameservers": nameservers},
            "publishedPorts": published_ports
        },
        "id": format!("soma-{INSTANCE}"),
        "status": {"networks": active_networks}
    }]))
    .expect("valid inspection fixture")
}

fn inspect(document: Vec<u8>) -> crate::InspectedMachine {
    let (backend, _) = backend([Ok(success(document))]);
    backend
        .inspect(instance(), control_limits())
        .expect("ownership is exact")
}

#[test]
fn inspect_reports_network_attachment_only_when_configured_and_active_sets_agree() {
    let attached = inspection_document(
        &json!([{"network": "default"}]),
        &json!([{"network": "default"}]),
        &json!([]),
        &json!([]),
    );
    let detached = inspection_document(&json!([]), &json!([]), &json!([]), &json!([]));
    let inconsistent = inspection_document(
        &json!([]),
        &json!([{"network": "default"}]),
        &json!([]),
        &json!([]),
    );

    for (document, expected) in [
        (attached, Some(NetworkAttachment::Attached)),
        (detached, Some(NetworkAttachment::Detached)),
        (inconsistent, None),
    ] {
        assert_eq!(inspect(document).network_attachment(), expected);
    }
}

#[test]
fn inspect_reports_exact_apple_container_1_3_network_evidence() {
    let document = inspection_document(
        &json!([{"network": "default"}]),
        &json!([{
            "network": "default",
            "ipv4Address": "192.168.64.7/24",
            "ipv6Address": "fd00::7/64"
        }]),
        &json!(["2606:4700:4700::1111", "1.1.1.1"]),
        &json!([
            {
                "containerPort": 53,
                "count": 1,
                "hostAddress": "127.0.0.1",
                "hostPort": 5353,
                "proto": "udp"
            },
            {
                "containerPort": 3000,
                "count": 1,
                "hostAddress": "127.0.0.1",
                "hostPort": 3000,
                "proto": "tcp"
            }
        ]),
    );
    let inspection = inspect(document);
    let network = inspection.network();

    assert_eq!(network.attachment(), Some(NetworkAttachment::Attached));
    assert_eq!(
        network.dns_servers(),
        Some(
            [
                "1.1.1.1".parse::<IpAddr>().expect("IPv4 DNS"),
                "2606:4700:4700::1111".parse::<IpAddr>().expect("IPv6 DNS"),
            ]
            .as_slice()
        )
    );
    let expected_ports = [
        PublishedPort::new(Ipv4Addr::LOCALHOST, 3_000, 3_000, TransportProtocol::Tcp)
            .expect("valid TCP publication"),
        PublishedPort::new(Ipv4Addr::LOCALHOST, 5_353, 53, TransportProtocol::Udp)
            .expect("valid UDP publication"),
    ];
    assert_eq!(network.published_ports(), Some(expected_ports.as_slice()));
    let addresses = network.addresses().expect("exact active addresses");
    assert_eq!(addresses.len(), 2);
    assert_eq!(
        (addresses[0].address(), addresses[0].prefix_length()),
        ("192.168.64.7".parse::<IpAddr>().expect("IPv4 address"), 24)
    );
    assert_eq!(
        (addresses[1].address(), addresses[1].prefix_length()),
        ("fd00::7".parse::<IpAddr>().expect("IPv6 address"), 64)
    );
}

#[test]
fn inspect_fails_closed_for_malformed_published_port_records() {
    for publication in [
        json!({
            "containerPort": 3000,
            "count": 2,
            "hostAddress": "127.0.0.1",
            "hostPort": 3000,
            "proto": "tcp"
        }),
        json!({
            "containerPort": 3000,
            "count": 1,
            "hostAddress": "127.0.0.1",
            "hostPort": 3000,
            "proto": "sctp"
        }),
    ] {
        let inspection = inspect(inspection_document(
            &json!([{"network": "default"}]),
            &json!([{"network": "default"}]),
            &json!([]),
            &json!([publication]),
        ));

        assert_eq!(inspection.network().published_ports(), None);
    }
}

#[test]
fn inspect_fails_closed_for_malformed_cidr_or_network_set_mismatch() {
    let malformed_cidr = inspect(inspection_document(
        &json!([{"network": "default"}]),
        &json!([{"network": "default", "ipv4Address": "192.168.64.7/33"}]),
        &json!([]),
        &json!([]),
    ));
    assert_eq!(malformed_cidr.network().addresses(), None);

    let mismatched = inspect(inspection_document(
        &json!([{"network": "default"}]),
        &json!([{"network": "private", "ipv4Address": "192.168.64.7/24"}]),
        &json!([]),
        &json!([]),
    ));
    assert_eq!(mismatched.network().attachment(), None);
    assert_eq!(mismatched.network().addresses(), None);
}

#[test]
fn owned_inspection_without_valid_resource_fields_reports_them_unavailable() {
    let document = format!(
        r#"[{{"configuration":{{"id":"soma-{INSTANCE}","labels":{{"io.miosa.soma.instance":"{INSTANCE}"}}}},"id":"soma-{INSTANCE}"}}]"#
    );
    let (backend, runner) = backend([Ok(success(document.into_bytes()))]);

    let inspection = backend
        .inspect(instance(), control_limits())
        .expect("ownership is still exact");

    assert_eq!(inspection.resources(), None);
    assert_eq!(runner.calls().len(), 1);
}
