use super::{Build, ParsedReport, Protocol, RELEASE, Source, Speaks, report, version_requested};

const COMMIT: &str = "2ec3789c4f1a8e9d0b1c2d3e4f5a6b7c8d9e0f1a";
const DATE: &str = "2026-11-20T18:04:11Z";

#[test]
fn a_clean_build_reports_every_fact_it_was_given() {
    let build = Build::from_environment(Some(COMMIT), Some(DATE), Some("clean"));

    assert_eq!(build.commit(), COMMIT);
    assert_eq!(build.commit_short(), "2ec3789c4f1a");
    assert_eq!(build.commit_date(), DATE);
    assert_eq!(build.source(), Source::Clean);
    assert_eq!(build.revision(), "2ec3789c4f1a");
}

#[test]
fn a_dirty_build_is_marked_dirty_in_its_revision() {
    let build = Build::from_environment(Some(COMMIT), Some(DATE), Some("dirty"));

    assert_eq!(build.source(), Source::Dirty);
    assert_eq!(build.revision(), "2ec3789c4f1a-dirty");
    assert_eq!(
        build.commit(),
        COMMIT,
        "the full commit stays a plain commit"
    );
}

#[test]
fn a_build_without_a_valid_commit_claims_nothing() {
    for commit in [
        None,
        Some(""),
        Some("2ec3789"),
        Some("2EC3789C4F1A8E9D0B1C2D3E4F5A6B7C8D9E0F1A"),
        Some("zec3789c4f1a8e9d0b1c2d3e4f5a6b7c8d9e0f1a"),
    ] {
        let build = Build::from_environment(commit, Some(DATE), Some("clean"));

        assert_eq!(build, Build::unknown(), "{commit:?}");
        assert_eq!(build.commit(), "unknown");
        assert_eq!(build.commit_short(), "unknown");
        assert_eq!(build.commit_date(), "unknown");
        assert_eq!(build.source(), Source::Unknown);
    }
}

#[test]
fn a_sha256_commit_is_accepted() {
    let commit = "a".repeat(64).leak();
    let build = Build::from_environment(Some(commit), None, Some("clean"));

    assert_eq!(build.commit(), commit);
    assert_eq!(build.commit_short(), "aaaaaaaaaaaa");
}

#[test]
fn an_unrecognized_date_or_source_is_not_believed() {
    for date in [
        "2026-11-20",
        "2026-11-20T18:04:11+00:00",
        "2026-11-20 18:04:11Z",
    ] {
        let build = Build::from_environment(Some(COMMIT), Some(date), Some("clean"));
        assert_eq!(build.commit_date(), "unknown", "{date}");
    }
    let build = Build::from_environment(Some(COMMIT), Some(DATE), Some("mostly clean"));
    assert_eq!(build.source(), Source::Unknown);
    assert_eq!(build.revision(), "2ec3789c4f1a");
}

#[test]
fn the_report_names_the_build_and_every_contract() {
    let build = Build::from_environment(Some(COMMIT), Some(DATE), Some("dirty"));
    let protocols = [Protocol::new("hostd", 1), Protocol::new("guest-api", 2)];
    let speaks = Speaks {
        api: Some("1.0"),
        protocols: &protocols,
    };

    let text = report("soma-api", &build, &speaks);

    let expected = format!(
        "soma-api {RELEASE} (2ec3789c4f1a-dirty 2026-11-20)\n\
         release: {RELEASE}\n\
         commit: {COMMIT}\n\
         commit-short: 2ec3789c4f1a\n\
         commit-date: {DATE}\n\
         source: dirty\n\
         profile: {}\n\
         target: {}\n\
         api: 1.0\n\
         protocol.hostd: 1\n\
         protocol.guest-api: 2\n",
        build.profile(),
        build.target(),
    );
    assert_eq!(text, expected);
}

#[test]
fn an_unknown_build_says_so_on_its_first_line() {
    let text = report("soma-hostd", &Build::unknown(), &Speaks::default());

    assert_eq!(
        text.lines().next(),
        Some(format!("soma-hostd {RELEASE} (unknown)").as_str())
    );
    assert!(text.contains("\nsource: unknown\n"));
    assert!(
        !text.contains("api:"),
        "a binary that speaks no API prints none"
    );
}

#[test]
fn the_profile_and_target_are_what_cargo_compiled() {
    assert!(matches!(super::PROFILE, "debug" | "release"));
    assert!(super::TARGET.contains('-'), "{}", super::TARGET);
    assert_eq!(RELEASE, env!("CARGO_PKG_VERSION"));
}

#[test]
fn only_a_lone_version_flag_is_a_version_request() {
    assert!(version_requested(["soma-api", "--version"]));
    assert!(!version_requested(["soma-api"]));
    assert!(!version_requested(["soma-api", "--version", "--listen"]));
    assert!(!version_requested(["soma-api", "--listen", "--version"]));
    assert!(!version_requested(["soma-api", "version"]));
    assert!(!version_requested(["soma-api", "-V"]));
}

#[test]
fn the_macro_reads_the_environment_of_its_caller() {
    // This test crate was compiled without the identity script unless the run set it, and the
    // macro must agree with whatever `option_env!` sees here either way.
    let expected = Build::from_environment(
        option_env!("SOMA_BUILD_COMMIT"),
        option_env!("SOMA_BUILD_COMMIT_DATE"),
        option_env!("SOMA_BUILD_SOURCE"),
    );
    assert_eq!(crate::build!(), expected);
}

#[test]
fn a_report_reads_back_as_the_facts_it_was_written_from() {
    let build = Build::from_environment(Some(COMMIT), Some(DATE), Some("clean"));
    let protocols = [Protocol::new("netd", 1), Protocol::new("hostd", 4)];
    let speaks = Speaks {
        api: Some("1.2"),
        protocols: &protocols,
    };

    let parsed = ParsedReport::parse(&report("soma-api", &build, &speaks)).expect("parses");

    assert_eq!(parsed.binary, "soma-api");
    assert_eq!(parsed.value("release"), Some(RELEASE));
    assert_eq!(parsed.value("commit"), Some(COMMIT));
    assert_eq!(parsed.value("source"), Some("clean"));
    assert_eq!(parsed.value("missing"), None);
    assert_eq!(parsed.api.as_deref(), Some("1.2"));
    assert_eq!(
        parsed.protocols,
        [("netd".to_owned(), 1), ("hostd".to_owned(), 4)]
    );
}

#[test]
fn a_malformed_report_is_refused() {
    let good = report("soma-netd", &Build::unknown(), &Speaks::default());
    let reordered = good.replacen("release:", "version:", 1);
    for text in [
        "",
        "soma-netd",
        reordered.as_str(),
        &format!("{good}surprise: 1\n"),
        &format!("{good}protocol.netd: one\n"),
    ] {
        assert!(ParsedReport::parse(text).is_err(), "{text:?}");
    }
    assert!(ParsedReport::parse(&good).is_ok());
}
