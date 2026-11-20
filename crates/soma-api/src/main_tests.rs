//! The command-line options `soma-api` parses before it opens anything.

use super::Options;

#[test]
fn rejects_an_option_without_a_value() {
    assert!(Options::parse(["--listen".to_owned()].into_iter()).is_none());
}

#[test]
fn defaults_to_loopback_when_no_address_is_given() {
    let options = Options::parse(std::iter::empty()).expect("empty arguments parse");

    assert_eq!(options.listen, "127.0.0.1:8787");
    assert_eq!(options.workers, 128);
    assert!(
        options.runner_config.is_none(),
        "the public runner is off by default"
    );
}

#[test]
fn accepts_a_bounded_worker_count() {
    let options = Options::parse(["--workers".to_owned(), "100".to_owned()].into_iter())
        .expect("bounded worker count parses");

    assert_eq!(options.workers, 100);
}

#[test]
fn rejects_zero_or_unbounded_worker_counts() {
    for count in ["0", "1025", "not-a-number"] {
        assert!(Options::parse(["--workers".to_owned(), count.to_owned()].into_iter()).is_none());
    }
}
