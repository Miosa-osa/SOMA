use super::{ownership::require_facts, *};

use crate::Accepted;

fn current() -> (u32, u32) {
    // SAFETY: `getgid` reads process identity and has no preconditions.
    (broker_owner(), unsafe { libc::getgid() })
}

fn authority() -> ControlAuthority {
    let (uid, gid) = current();
    ControlAuthority::new(uid, gid, &[uid], &[uid]).expect("authority")
}

fn connect(path: &Path) -> OwnedFd {
    let target = c_path(path.as_os_str()).expect("path");
    let client = socket().expect("client socket");
    // SAFETY: `sockaddr_un` is a plain C aggregate for which all-zero bytes are valid.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(target.as_bytes()) {
        *slot = *byte as libc::c_char;
    }
    // SAFETY: `address` is fully initialised and its exact size is passed.
    let connected = unsafe {
        libc::connect(
            client.as_raw_fd(),
            (&raw const address).cast(),
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    assert_eq!(connected, 0, "client connect");
    client
}

fn read_byte(client: &OwnedFd) -> isize {
    let mut byte = [0_u8; 1];
    // SAFETY: `byte` is a valid writable buffer of exactly the passed length.
    unsafe { libc::recv(client.as_raw_fd(), byte.as_mut_ptr().cast(), byte.len(), 0) }
}

#[test]
fn the_socket_and_its_directory_are_owned_and_fail_closed_on_drift() {
    let root = tempfile::tempdir().expect("state dir");
    let path = root.path().join("run").join("broker.sock");
    let listener = ControlListener::bind(&path, authority()).expect("bind");
    let (uid, gid) = current();

    let directory = c_path(path.parent().expect("parent").as_os_str()).expect("directory");
    assert_eq!(
        facts(&directory).expect("stat").expect("present"),
        (libc::S_IFDIR | DIRECTORY_MODE, uid, gid)
    );
    let node = c_path(path.as_os_str()).expect("socket");
    assert_eq!(
        facts(&node).expect("stat").expect("present"),
        (libc::S_IFSOCK | SOCKET_MODE, uid, gid)
    );
    assert_eq!(listener.authority(), &authority());

    // SAFETY: the path is a valid NUL-terminated string owned by this process.
    assert_eq!(unsafe { libc::chmod(node.as_ptr(), 0o666) }, 0);
    assert_eq!(
        listener.accept().expect_err("permission drift"),
        Error::Unauthorized("socket mode")
    );
    // SAFETY: the path is a valid NUL-terminated string owned by this process.
    assert_eq!(unsafe { libc::chmod(node.as_ptr(), SOCKET_MODE) }, 0);
    // SAFETY: the path is a valid NUL-terminated string owned by this process.
    assert_eq!(unsafe { libc::chmod(directory.as_ptr(), 0o777) }, 0);
    assert_eq!(
        listener.accept().expect_err("directory drift"),
        Error::Unauthorized("socket directory mode")
    );
}

#[test]
fn ownership_and_mode_decisions_refuse_every_drifted_node() {
    let authority = ControlAuthority::new(7, 8, &[7], &[]).expect("authority");

    assert_eq!(
        require_facts(
            Node::Socket,
            (libc::S_IFSOCK | SOCKET_MODE, 7, 8),
            &authority
        ),
        Ok(())
    );
    assert_eq!(
        require_facts(
            Node::Socket,
            (libc::S_IFREG | SOCKET_MODE, 7, 8),
            &authority
        ),
        Err(Error::Unauthorized("socket type"))
    );
    assert_eq!(
        require_facts(
            Node::Socket,
            (libc::S_IFSOCK | SOCKET_MODE, 9, 8),
            &authority
        ),
        Err(Error::Unauthorized("socket owner"))
    );
    assert_eq!(
        require_facts(
            Node::Socket,
            (libc::S_IFSOCK | SOCKET_MODE, 7, 9),
            &authority
        ),
        Err(Error::Unauthorized("socket owner"))
    );
    assert_eq!(
        require_facts(Node::Socket, (libc::S_IFSOCK | 0o666, 7, 8), &authority),
        Err(Error::Unauthorized("socket mode"))
    );
    assert_eq!(
        require_facts(
            Node::Directory,
            (libc::S_IFDIR | DIRECTORY_MODE, 7, 8),
            &authority
        ),
        Ok(())
    );
    assert_eq!(
        require_facts(Node::Directory, (libc::S_IFDIR | 0o777, 7, 8), &authority),
        Err(Error::Unauthorized("socket directory mode"))
    );
    assert_eq!(
        require_facts(
            Node::Directory,
            (libc::S_IFDIR | DIRECTORY_MODE, 9, 8),
            &authority
        ),
        Err(Error::Unauthorized("socket directory owner"))
    );
}

#[test]
fn a_restart_replaces_its_own_socket_and_refuses_any_other_stale_path() {
    let root = tempfile::tempdir().expect("state dir");
    let path = root.path().join("run").join("broker.sock");
    let first = ControlListener::bind(&path, authority()).expect("first bind");
    drop(first);

    let second = ControlListener::bind(&path, authority()).expect("restart over its own socket");
    drop(second);

    std::fs::remove_file(&path).expect("remove socket");
    std::fs::write(&path, b"not a socket").expect("stale regular file");
    assert_eq!(
        ControlListener::bind(&path, authority()).expect_err("stale file"),
        Error::Unauthorized("stale socket path")
    );
}

#[test]
fn an_unadmitted_peer_is_closed_before_it_can_send_or_receive_anything() {
    let root = tempfile::tempdir().expect("state dir");
    let path = root.path().join("run").join("broker.sock");
    let (uid, gid) = current();
    let closed =
        ControlAuthority::new(uid, gid, &[uid.wrapping_add(1)], &[]).expect("foreign authority");
    let listener = ControlListener::bind(&path, closed).expect("bind");

    let client = connect(&path);
    match listener.accept().expect("accept") {
        Accepted::Rejected(peer) => {
            assert_eq!(peer.uid(), uid);
            assert_eq!(peer.gid(), gid);
            // SAFETY: `getpid` reads process identity and has no preconditions.
            assert_eq!(peer.pid(), unsafe { libc::getpid() });
        }
        Accepted::Authorized(..) => panic!("an unadmitted peer must not be authorized"),
    }
    assert_eq!(read_byte(&client), 0, "the connection must be closed");
}

#[test]
fn an_admitted_peer_carries_its_kernel_derived_identity() {
    let root = tempfile::tempdir().expect("state dir");
    let path = root.path().join("run").join("broker.sock");
    let listener = ControlListener::bind(&path, authority()).expect("bind");
    let (uid, gid) = current();

    let client = connect(&path);
    let Accepted::Authorized(connection, peer) = listener.accept().expect("accept") else {
        panic!("an admitted peer must be authorized");
    };
    assert_eq!((peer.uid(), peer.gid()), (uid, gid));

    let byte = [7_u8; 1];
    // SAFETY: `byte` is a valid buffer for its full length.
    let sent = unsafe {
        libc::send(
            connection.as_raw_fd(),
            byte.as_ptr().cast(),
            byte.len(),
            libc::MSG_NOSIGNAL,
        )
    };
    assert_eq!(sent, 1);
    assert_eq!(read_byte(&client), 1);
}
