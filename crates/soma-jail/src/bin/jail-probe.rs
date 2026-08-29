//! The test stand-in for `soma-vmm`: reports what it can see from inside the jail and executes
//! containment commands sent over the control socket.
//!
//! It is only meaningful on Linux `x86_64` inside a jail; elsewhere it exits with status 2.

#![deny(warnings)]

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod probe {
    #![allow(unsafe_code)]

    use std::{fs, io, thread};

    use soma_jail::{
        DescriptorError, DescriptorManifest, DescriptorRole, Phase, ProbeCommand, ProbeReport,
        RootView, VerificationDepth, install_filter, verify_sealed_table,
    };

    const KVM_GET_API_VERSION: libc::Ioctl = 0xAE00;
    const TUNSETIFF: libc::Ioctl = 0x4004_54CA;

    fn errno() -> i32 {
        io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

    fn send(fd: libc::c_int, text: &str) {
        // SAFETY: the buffer and length describe a live string slice.
        unsafe { libc::send(fd, text.as_ptr().cast(), text.len(), 0) };
    }

    fn receive(fd: libc::c_int, buffer: &mut [u8; 64]) -> Option<usize> {
        // SAFETY: the buffer and length describe valid writable storage.
        let received = unsafe { libc::recv(fd, buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        usize::try_from(received).ok().filter(|count| *count > 0)
    }

    fn descriptor_limit() -> u32 {
        let mut rlimit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `RLIMIT_NOFILE` is a valid resource and `rlimit` outlives the call.
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut rlimit) };
        u32::try_from(rlimit.rlim_cur).unwrap_or(u32::MAX).min(4096)
    }

    fn report(manifest: &DescriptorManifest) -> ProbeReport {
        let verification =
            verify_sealed_table(manifest, VerificationDepth::Sealed, descriptor_limit());
        let first_bad_slot = verification.err().map(|error| match error {
            DescriptorError::Missing { slot, .. }
            | DescriptorError::Kind { slot, .. }
            | DescriptorError::Device { slot }
            | DescriptorError::NotSeqpacket { slot }
            | DescriptorError::Unexpected { slot }
            | DescriptorError::Dup { slot, .. } => slot,
            DescriptorError::CloseRange(_) => u32::MAX,
        });
        let root_entries = fs::read_dir("/").map_or(u32::MAX, |entries| {
            u32::try_from(entries.count()).unwrap_or(u32::MAX)
        });
        let root_writable = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open("/jail-probe")
            .is_ok();
        // SAFETY: identity getters take no arguments and cannot fail.
        let identity = unsafe {
            [
                libc::getuid(),
                libc::geteuid(),
                libc::getgid(),
                libc::getegid(),
            ]
        };
        ProbeReport {
            // SAFETY: `getpid` takes no arguments and cannot fail.
            pid: unsafe { libc::getpid() },
            uid: identity[0],
            euid: identity[1],
            gid: identity[2],
            egid: identity[3],
            table_sealed: first_bad_slot.is_none(),
            first_bad_slot,
            root: RootView {
                entries: root_entries,
                writable: root_writable,
                proc_visible: fs::metadata("/proc").is_ok(),
                sys_visible: fs::metadata("/sys").is_ok(),
            },
        }
    }

    fn spawn_threads(count: u32) -> (u32, i32) {
        let mut spawned = 0;
        let mut last_errno = 0;
        for _ in 0..count {
            match thread::Builder::new().spawn(|| {
                loop {
                    thread::park();
                }
            }) {
                Ok(_) => spawned += 1,
                Err(error) => {
                    last_errno = error.raw_os_error().unwrap_or(0);
                    break;
                }
            }
        }
        (spawned, last_errno)
    }

    fn allocate(mib: u32) {
        let bytes = usize::try_from(mib).unwrap_or(0) << 20;
        let mut block = vec![0u8; bytes];
        for offset in (0..bytes).step_by(4096) {
            block[offset] = 1;
        }
        std::hint::black_box(&block);
    }

    fn execute(command: ProbeCommand, control: libc::c_int, kvm: Option<libc::c_int>) {
        match command {
            ProbeCommand::Exit(code) => std::process::exit(code),
            ProbeCommand::Steady => match install_filter(Phase::SteadyState) {
                Ok(()) => send(control, "ok steady"),
                Err(error) => send(control, &format!("error {error}")),
            },
            ProbeCommand::Socket => {
                // SAFETY: `socket` takes integer arguments only; the filter is expected to kill.
                let result = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
                send(control, &format!("alive socket={result} errno={}", errno()));
            }
            ProbeCommand::ForbiddenIoctl => {
                let fd = kvm.unwrap_or(-1);
                // SAFETY: the filter is expected to kill the process before the kernel sees the
                // null argument, and `TUNSETIFF` on a KVM descriptor is rejected anyway.
                let result = unsafe { libc::ioctl(fd, TUNSETIFF, std::ptr::null_mut::<u8>()) };
                send(control, &format!("alive ioctl={result} errno={}", errno()));
            }
            ProbeCommand::KvmVersion => {
                let fd = kvm.unwrap_or(-1);
                // SAFETY: `KVM_GET_API_VERSION` takes no pointer; the kernel requires a zero
                // argument and returns `EINVAL` for anything else.
                let result = unsafe { libc::ioctl(fd, KVM_GET_API_VERSION, 0) };
                send(control, &format!("ok {result}"));
            }
            ProbeCommand::Threads(count) => {
                let (spawned, last_errno) = spawn_threads(count);
                send(control, &format!("ok {spawned} {last_errno}"));
            }
            ProbeCommand::Allocate(mib) => {
                allocate(mib);
                send(control, "ok allocated");
            }
            ProbeCommand::Exec => {
                let argv: [*const libc::c_char; 2] = [c"/bin/true".as_ptr(), std::ptr::null()];
                // SAFETY: `argv` is null-terminated and the path is a valid C string; the
                // filter is expected to kill the process.
                let result = unsafe {
                    libc::execve(c"/bin/true".as_ptr(), argv.as_ptr(), argv[1..].as_ptr())
                };
                send(control, &format!("alive execve={result} errno={}", errno()));
            }
            ProbeCommand::CreateFile => {
                let result = fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open("/created");
                let code = result
                    .err()
                    .and_then(|error| error.raw_os_error())
                    .unwrap_or(0);
                send(control, &format!("ok {code}"));
            }
        }
    }

    pub fn main() -> i32 {
        let Some(encoded) = std::env::args().nth(1) else {
            return 2;
        };
        let Ok(manifest) = DescriptorManifest::decode(&encoded) else {
            return 2;
        };
        let Some(control) = manifest
            .slot_for(DescriptorRole::Control)
            .and_then(|slot| libc::c_int::try_from(slot).ok())
        else {
            return 2;
        };
        let kvm = manifest
            .slot_for(DescriptorRole::Kvm)
            .and_then(|slot| libc::c_int::try_from(slot).ok());
        send(control, &report(&manifest).encode());
        let mut buffer = [0u8; 64];
        while let Some(count) = receive(control, &mut buffer) {
            let text = String::from_utf8_lossy(&buffer[..count]).into_owned();
            match ProbeCommand::decode(&text) {
                Ok(command) => execute(command, control, kvm),
                Err(error) => send(control, &format!("error {error}")),
            }
        }
        0
    }
}

fn main() {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    std::process::exit(probe::main());
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    std::process::exit(2);
}
