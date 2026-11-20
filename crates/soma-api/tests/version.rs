//! `soma-api --version` reports this build and the contracts the binary speaks.

#[test]
fn version_reports_this_build_and_the_contracts_it_speaks() {
    let api = soma_api::API_VERSION.to_string();
    let protocols = soma_api::version::protocols();
    let speaks = soma_version::Speaks {
        api: Some(&api),
        protocols: &protocols,
    };
    soma_version::verify_binary(
        std::path::Path::new(env!("CARGO_BIN_EXE_soma-api")),
        "soma-api",
        &soma_version::build!(),
        &speaks,
    )
    .unwrap_or_else(|failure| panic!("{failure}"));
}
