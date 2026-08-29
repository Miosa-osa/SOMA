//! The "public Internet" stand-in: one namespace behind the broker's uplink that answers on
//! a public documentation address, on the declared and undeclared resolver addresses, and on
//! the cloud metadata address, so every drop in the live test is a policy decision rather
//! than a missing route.

use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, UdpSocket},
    path::Path,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use soma_netd::NetNamespace;

pub const UPLINK: &str = "uplink0";
pub const HOST_ADDRESS: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 1);
pub const PUBLIC_ADDRESS: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 10);
pub const PUBLIC_PORT: u16 = 8080;
pub const DECLARED_RESOLVER: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
pub const UNDECLARED_RESOLVER: Ipv4Addr = Ipv4Addr::new(8, 8, 8, 8);
pub const METADATA: Ipv4Addr = Ipv4Addr::new(169, 254, 169, 254);
const NETNS_DIR: &str = "/var/run/netns";
const NAME: &str = "soma-world";

pub struct World {
    stop: Arc<AtomicBool>,
    peers: Arc<Mutex<Vec<SocketAddr>>>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl World {
    pub fn build() -> Self {
        let _ = Command::new("ip").args(["netns", "del", NAME]).output();
        for args in [
            vec!["netns", "add", NAME],
            vec![
                "link", "add", UPLINK, "type", "veth", "peer", "name", "w0", "netns", NAME,
            ],
            vec!["-n", NAME, "addr", "add", "203.0.113.10/24", "dev", "w0"],
            vec!["-n", NAME, "addr", "add", "1.1.1.1/32", "dev", "w0"],
            vec!["-n", NAME, "addr", "add", "8.8.8.8/32", "dev", "w0"],
            vec!["-n", NAME, "addr", "add", "169.254.169.254/32", "dev", "w0"],
            vec!["-n", NAME, "link", "set", "w0", "up"],
            vec!["-n", NAME, "link", "set", "lo", "up"],
            vec!["-n", NAME, "route", "add", "default", "via", "203.0.113.1"],
            vec!["addr", "add", "203.0.113.1/24", "dev", UPLINK],
            vec!["link", "set", UPLINK, "up"],
            vec!["route", "add", "1.1.1.1/32", "via", "203.0.113.10"],
            vec!["route", "add", "8.8.8.8/32", "via", "203.0.113.10"],
            vec!["route", "add", "169.254.169.254/32", "via", "203.0.113.10"],
        ] {
            let output = Command::new("ip").args(&args).output().expect("ip binary");
            assert!(
                output.status.success(),
                "ip {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let namespace =
            NetNamespace::open(&Path::new(NETNS_DIR).join(NAME)).expect("world namespace");
        let (tcp, metadata, declared, undeclared) = namespace
            .within(|| {
                Ok((
                    TcpListener::bind((PUBLIC_ADDRESS, PUBLIC_PORT)).expect("public listener"),
                    TcpListener::bind((METADATA, 80)).expect("metadata listener"),
                    UdpSocket::bind((DECLARED_RESOLVER, 53)).expect("declared resolver"),
                    UdpSocket::bind((UNDECLARED_RESOLVER, 53)).expect("undeclared resolver"),
                ))
            })
            .expect("world sockets");
        let stop = Arc::new(AtomicBool::new(false));
        let peers = Arc::new(Mutex::new(Vec::new()));
        let mut threads = Vec::new();
        for listener in [tcp, metadata] {
            listener.set_nonblocking(true).expect("nonblocking");
            let stop = Arc::clone(&stop);
            let peers = Arc::clone(&peers);
            threads.push(thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, peer)) => {
                            peers.lock().expect("peers").push(peer);
                            let _ = stream.write_all(b"world");
                            let mut sink = [0; 64];
                            let _ = stream.read(&mut sink);
                        }
                        Err(_) => thread::sleep(Duration::from_millis(5)),
                    }
                }
            }));
        }
        for socket in [declared, undeclared] {
            socket
                .set_read_timeout(Some(Duration::from_millis(20)))
                .expect("timeout");
            let stop = Arc::clone(&stop);
            threads.push(thread::spawn(move || {
                let mut buffer = [0; 512];
                while !stop.load(Ordering::Relaxed) {
                    if let Ok((_, peer)) = socket.recv_from(&mut buffer) {
                        let _ = socket.send_to(b"dns-ok", peer);
                    }
                }
            }));
        }
        Self {
            stop,
            peers,
            threads,
        }
    }

    /// Returns the accepted peers, waiting briefly for the listener thread to record one.
    pub fn accepted_peers(&self) -> Vec<SocketAddr> {
        for _ in 0..50 {
            let peers = self.peers.lock().expect("peers").clone();
            if !peers.is_empty() {
                return peers;
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.peers.lock().expect("peers").clone()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        // The uplink end lives in the host namespace; deleting it synchronously removes the
        // pair instead of waiting for the kernel's deferred namespace teardown.
        let _ = Command::new("ip").args(["link", "del", UPLINK]).output();
        let _ = Command::new("ip").args(["netns", "del", NAME]).output();
    }
}
