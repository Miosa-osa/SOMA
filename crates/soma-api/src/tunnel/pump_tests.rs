use std::{
    net::TcpStream,
    os::unix::net::UnixStream,
    thread,
    time::{Duration, Instant},
};

use super::pump_blocking;

#[test]
fn the_blocking_pump_ends_when_the_guest_ends_and_reports_every_byte() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let (guest_end, mut guest) = UnixStream::pair().expect("pair");
    let server = thread::spawn(move || {
        let (client, _) = listener.accept().expect("accept");
        pump_blocking(&client, &guest_end, Duration::from_secs(30));
    });
    let mut client = TcpStream::connect(address).expect("connect");
    client.write_all(b"hello").expect("write");
    let mut got = [0_u8; 5];
    guest.read_exact(&mut got).expect("guest reads");
    assert_eq!(&got, b"hello");
    guest.write_all(b"world").expect("guest writes");
    let mut back = [0_u8; 5];
    client.read_exact(&mut back).expect("client reads");
    assert_eq!(&back, b"world");
    drop(guest);
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).expect("EOF");
    assert!(rest.is_empty());
    server.join().expect("pump ends once the guest has");
}

#[test]
fn the_blocking_pump_ends_an_idle_tunnel() {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let (guest_end, _guest) = UnixStream::pair().expect("pair");
    let server = thread::spawn(move || {
        let (client, _) = listener.accept().expect("accept");
        let started = Instant::now();
        pump_blocking(&client, &guest_end, Duration::from_secs(2));
        started.elapsed()
    });
    let _client = TcpStream::connect(address).expect("connect");
    let took = server.join().expect("pump ends");
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(6),
        "{took:?}"
    );
}
