#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    num::NonZeroUsize,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, channel},
    },
    thread,
    time::{Duration, Instant},
};

use soma_api::{ApiError, CreateAdmission, serve};
use support::{FakeFacade, Mode, anonymous, identified};

/// Starts the real accept loop on a loopback port with a create limit of one.
///
/// The first facade the service opens waits until `release` is signalled, so the create that
/// opened it holds the only admission place for as long as the test wants it held. Every later
/// facade opens at once.
fn service_with_one_create_place(release: Receiver<()>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let address = listener.local_addr().expect("bound address").to_string();
    let release = Arc::new(Mutex::new(Some(release)));
    let open_facade = move || {
        // Taken before waiting, so the lock is not held while this facade waits.
        let first = release.lock().expect("release lock").take();
        if let Some(release) = first {
            let _ignored = release.recv();
        }
        Ok::<_, ApiError>(FakeFacade::new(Mode::Succeed))
    };
    let admission = CreateAdmission::new(NonZeroUsize::MIN);
    thread::spawn(move || serve(&listener, open_facade, &admission));
    address
}

/// Sends one request on its own connection and returns the raw response.
fn exchange(address: &str, raw: &str) -> String {
    let mut stream = TcpStream::connect(address).expect("connect");
    let closing = raw.replacen(
        "host: localhost\r\n",
        "host: localhost\r\nconnection: close\r\n",
        1,
    );
    stream.write_all(closing.as_bytes()).expect("send");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("receive");
    response
}

fn status(response: &str) -> u16 {
    response
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status line")
}

fn create() -> String {
    identified("POST", "/v1/sandboxes", r#"{"image":"node:22"}"#)
}

#[test]
fn a_create_over_the_limit_is_refused_at_once_and_other_routes_still_serve() {
    let (release, held) = channel();
    let address = service_with_one_create_place(held);

    // The first create takes the only place and waits inside the service.
    let first = {
        let address = address.clone();
        thread::spawn(move || exchange(&address, &create()))
    };
    thread::sleep(Duration::from_millis(100));

    let started = Instant::now();
    let refused = exchange(&address, &create());
    let waited = started.elapsed();

    assert_eq!(status(&refused), 503, "{refused}");
    assert!(refused.contains("\r\nretry-after: 0\r\n"), "{refused}");
    assert!(refused.contains(r#""code":"runtime_busy""#), "{refused}");
    assert!(refused.contains(r#""retryable":true"#), "{refused}");
    // Refused without waiting for a facade, which would take the pool's one-second timeout.
    assert!(waited < Duration::from_millis(500), "waited {waited:?}");

    // A route that is not a create is not counted against the limit.
    let listed = exchange(&address, &identified("GET", "/v1/sandboxes", ""));
    assert_eq!(status(&listed), 200, "{listed}");
    // An unidentified create learns it is unidentified, not that the runner is busy.
    let unidentified = exchange(&address, &anonymous("POST", "/v1/sandboxes", "{}"));
    assert_eq!(status(&unidentified), 401, "{unidentified}");

    release.send(()).expect("release the first create");
    let first = first.join().expect("first create completed");
    assert_eq!(status(&first), 201, "{first}");
    assert!(!first.contains("retry-after"), "{first}");

    // The finished create gave its place back.
    let next = exchange(&address, &create());
    assert_eq!(status(&next), 201, "{next}");
}
