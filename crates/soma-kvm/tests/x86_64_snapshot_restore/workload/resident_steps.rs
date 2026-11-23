//! Drive a resident process through pause, resume and checkpoint, and read its state back.

use super::{SandboxMachine, Session, execute, session};

pub(super) fn exercise_resident<'a>(
    machine: &'a SandboxMachine,
    session: Session<'a>,
    lifecycle: impl FnOnce(&SandboxMachine, &str) -> Result<(), String>,
) -> Result<(Session<'a>, (u64, u64)), String> {
    use crate::x86_64_snapshot_restore_instance::command;
    let node = b"/usr/local/bin/node";
    let child = br"
            const crypto = require('node:crypto');
            const net = require('node:net');
            const marker = crypto.randomBytes(16).toString('hex');
            let ticks = 0;
            setInterval(() => ticks++, 5);
            net.createServer(c => c.end([process.pid, marker, ticks].join(' ')))
                .listen('/tmp/soma-resident-proof.sock');
        ";
    // Register a long-lived workload through the guest agent's existing process policy.
    let start = br"
            const fs = require('node:fs');
            const child = require('node:child_process').spawn(
                process.execPath, ['-e', process.argv[1]], {detached: true, stdio: 'ignore'});
            fs.mkdirSync('/run/soma', {recursive: true});
            fs.writeFileSync('/run/soma/survivors', String(child.pid) + '\n');
            child.unref();
        ";
    let probe = br"
            const net = require('node:net');
            let attempts = 0;
            function connect() {
                const client = net.connect('/tmp/soma-resident-proof.sock');
                client.on('data', chunk => process.stdout.write(chunk));
                client.on('error', error => {
                    if (++attempts < 200) setTimeout(connect, 10);
                    else throw error;
                });
            }
            connect();
        ";
    let (session, started) = execute(session, &command(node, &[b"-e", start, child]))?;
    assert_eq!(
        started.status,
        soma_guest::TerminalStatus::Exited(0),
        "start resident Node workload: {}",
        String::from_utf8_lossy(&started.stderr),
    );
    let (session, before) = execute(session, &command(node, &[b"-e", probe]))?;
    let before = resident_state(&before, "before pause");
    lifecycle(machine, before.1)?;
    assert!(!machine.is_paused());
    let (session, after) = execute(session, &command(node, &[b"-e", probe]))?;
    let after = resident_state(&after, "after resume");
    assert_eq!(
        before.0, after.0,
        "the original Linux process should still exist"
    );
    assert_eq!(
        before.1, after.1,
        "the original in-memory marker should survive"
    );
    assert!(
        after.2 > before.2,
        "the resident process should make progress"
    );
    Ok((session, (before.2, after.2)))
}

pub(super) fn find_in_image(file: &mut std::fs::File, needle: &[u8]) -> bool {
    use std::io::Read as _;

    let mut bytes = vec![0_u8; 1 << 20];
    let mut previous = 0;
    loop {
        let read = file
            .read(&mut bytes[previous..])
            .expect("read checkpoint RAM");
        if read == 0 {
            return false;
        }
        let total = previous + read;
        if bytes[..total]
            .windows(needle.len())
            .any(|window| window == needle)
        {
            return true;
        }
        previous = total.min(needle.len() - 1);
        bytes.copy_within(total - previous..total, 0);
    }
}

pub(super) fn resident_state<'a>(
    output: &'a session::Executed,
    phase: &str,
) -> (u32, &'a str, u64) {
    assert_eq!(
        output.status,
        soma_guest::TerminalStatus::Exited(0),
        "process probe failed {phase}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let text = std::str::from_utf8(&output.stdout).expect("UTF-8 process state");
    let mut values = text.split_ascii_whitespace();
    let pid = values.next().unwrap().parse().unwrap();
    let marker = values.next().unwrap();
    assert_eq!(marker.len(), 32);
    let ticks = values.next().unwrap().parse().unwrap();
    assert!(values.next().is_none());
    (pid, marker, ticks)
}
