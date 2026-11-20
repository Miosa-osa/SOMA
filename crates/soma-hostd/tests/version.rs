//! `soma-hostd --version` reports this build and the contracts the binary speaks.

#[test]
fn version_reports_this_build_and_the_contracts_it_speaks() {
    let protocols = [
        soma_version::Protocol::new("hostd", u32::from(soma_hostd::protocol::PROTOCOL_VERSION)),
        soma_version::Protocol::new("netd", u32::from(soma_netd::PROTOCOL_VERSION)),
    ];
    let speaks = soma_version::Speaks {
        api: None,
        protocols: &protocols,
    };
    soma_version::verify_binary(
        std::path::Path::new(env!("CARGO_BIN_EXE_soma-hostd")),
        "soma-hostd",
        &soma_version::build!(),
        &speaks,
    )
    .unwrap_or_else(|failure| panic!("{failure}"));
}
