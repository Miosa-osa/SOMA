//! `soma-api --version` reports this build and the contracts the binary speaks.

#[test]
fn version_reports_this_build_and_the_contracts_it_speaks() {
    let protocols = soma_local::PROTOCOLS;
    let speaks = soma_version::Speaks {
        api: None,
        protocols,
    };
    soma_version::verify_binary(
        std::path::Path::new(env!("CARGO_BIN_EXE_soma-api")),
        "soma-api",
        &soma_version::build!(),
        &speaks,
    )
    .unwrap_or_else(|failure| panic!("{failure}"));
}
