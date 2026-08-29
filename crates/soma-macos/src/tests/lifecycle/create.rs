use std::net::{IpAddr, Ipv4Addr};

use crate::{
    DnsConfiguration, NetworkConfiguration, NetworkPolicy, PublishedPort, TransportProtocol,
};

use super::{
    super::fixtures::{INSTANCE, backend, strings, success},
    create_request,
};

#[test]
fn create_encodes_each_network_policy_explicitly() {
    let cases = [
        (NetworkPolicy::Unspecified, None),
        (NetworkPolicy::Denied, Some("none")),
        (NetworkPolicy::Allowed, Some("default")),
    ];

    for (policy, expected_network) in cases {
        let (backend, runner) = backend([Ok(success(Vec::<u8>::new()))]);
        let request = create_request().with_network_policy(policy);

        backend.create(&request).expect("create succeeds");

        let arguments = &runner.calls()[0].arguments;
        let network_position = arguments
            .iter()
            .position(|argument| argument == "--network");
        match expected_network {
            Some(network) => {
                let position = network_position.expect("explicit policy has a network flag");
                assert_eq!(
                    arguments.get(position + 1).map(String::as_str),
                    Some(network)
                );
            }
            None => assert_eq!(network_position, None),
        }
    }
}

#[test]
fn create_encodes_exact_custom_dns_and_tcp_udp_publication_arguments() {
    let dns = DnsConfiguration::custom(vec![
        "2606:4700:4700::1111".parse::<IpAddr>().expect("IPv6 DNS"),
        "1.1.1.1".parse::<IpAddr>().expect("IPv4 DNS"),
    ])
    .expect("valid exact DNS");
    let publications = vec![
        PublishedPort::new(Ipv4Addr::LOCALHOST, 5_353, 53, TransportProtocol::Udp)
            .expect("valid UDP publication"),
        PublishedPort::new(Ipv4Addr::LOCALHOST, 3_000, 3_000, TransportProtocol::Tcp)
            .expect("valid TCP publication"),
    ];
    let network = NetworkConfiguration::new(NetworkPolicy::Allowed, dns, publications)
        .expect("valid Apple network configuration");
    let request = create_request().with_network(network);
    let (backend, runner) = backend([Ok(success(Vec::<u8>::new()))]);

    backend.create(&request).expect("create succeeds");

    assert_eq!(
        runner.calls()[0].arguments,
        strings(&[
            "create",
            "--name",
            &format!("soma-{INSTANCE}"),
            "--label",
            &format!("io.miosa.soma.instance={INSTANCE}"),
            "--cpus",
            "1",
            "--memory",
            "1024M",
            "--network",
            "default",
            "--dns",
            "1.1.1.1",
            "--dns",
            "2606:4700:4700::1111",
            "--publish",
            "127.0.0.1:3000:3000/tcp",
            "--publish",
            "127.0.0.1:5353:53/udp",
            "--entrypoint",
            "/bin/sleep",
            "node:22",
            "infinity",
        ])
    );
}
