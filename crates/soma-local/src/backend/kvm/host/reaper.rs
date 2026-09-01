//! One bounded owner for machine-host child exit status.
//!
//! A successful Launch hands the child process to this module after its startup pipes are
//! consumed.
//! The machine keeps running independently, while one process-wide thread periodically reaps
//! every child that has exited.
//! Dropping a `Child` without this owner leaves a zombie under a long-running API process.

use std::{
    process::Child,
    sync::{OnceLock, mpsc},
    time::Duration,
};

const REAP_TICK: Duration = Duration::from_millis(10);
const ADOPTION_QUEUE: usize = 1024;

/// Transfers one successfully launched machine host to the process-wide reaper.
pub(super) fn adopt(child: Child) -> Result<(), AdoptionFailure> {
    adopt_with(sender(), child)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AdoptionFailure;

fn adopt_with(sender: &mpsc::SyncSender<Child>, child: Child) -> Result<(), AdoptionFailure> {
    match sender.try_send(child) {
        Ok(()) => Ok(()),
        Err(mpsc::TrySendError::Full(child)) => match sender.send(child) {
            Ok(()) => Ok(()),
            Err(mpsc::SendError(child)) => {
                terminate_and_wait(child);
                Err(AdoptionFailure)
            }
        },
        Err(mpsc::TrySendError::Disconnected(child)) => {
            terminate_and_wait(child);
            Err(AdoptionFailure)
        }
    }
}

fn sender() -> &'static mpsc::SyncSender<Child> {
    static SENDER: OnceLock<mpsc::SyncSender<Child>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(ADOPTION_QUEUE);
        std::thread::Builder::new()
            .name("soma-machine-reaper".to_owned())
            .spawn(move || run(&receiver))
            .expect("the machine-host reaper thread must start");
        sender
    })
}

fn run(receiver: &mpsc::Receiver<Child>) {
    let mut children = Vec::new();
    loop {
        match receiver.recv_timeout(REAP_TICK) {
            Ok(child) => children.push(child),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        children.extend(receiver.try_iter());
        let mut running = Vec::with_capacity(children.len());
        for mut child in children.drain(..) {
            match child.try_wait() {
                Ok(None) => running.push(child),
                Ok(Some(_)) => {}
                Err(_) => terminate_and_wait(child),
            }
        }
        children = running;
    }
    for mut child in children {
        let _ignored = child.wait();
    }
}

fn terminate_and_wait(mut child: Child) {
    let _ignored = child.kill();
    let _ignored = child.wait();
}

#[cfg(test)]
mod tests {
    use std::{process::Command, sync::mpsc, thread, time::Duration};

    #[test]
    fn an_adopted_child_is_reaped_after_it_exits() {
        let child = Command::new("sh")
            .args(["-c", "sleep 0.02"])
            .spawn()
            .expect("spawn child");
        let pid = child.id();

        super::adopt(child).expect("the process-wide reaper accepts children");

        for _ in 0..100 {
            if process_is_absent(pid) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("adopted child {pid} remained present after its exit");
    }

    #[test]
    fn a_full_queue_waits_for_adoption_instead_of_for_child_exit() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let first = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        sender.send(first).unwrap();
        let collector = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            for mut child in receiver.iter().take(2) {
                let _ignored = child.wait();
            }
        });
        let second = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();

        super::adopt_with(&sender, second).expect("collector keeps the queue connected");
        drop(sender);
        collector.join().unwrap();
    }

    #[test]
    fn a_disconnected_queue_terminates_and_collects_the_child() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(receiver);
        let child = Command::new("sh")
            .args(["-c", "sleep 60"])
            .spawn()
            .expect("spawn child");
        let pid = child.id();

        let result = super::adopt_with(&sender, child);

        assert_eq!(result, Err(super::AdoptionFailure));
        assert!(process_is_absent(pid));
    }

    #[cfg(target_os = "linux")]
    fn process_is_absent(pid: u32) -> bool {
        !std::path::Path::new(&format!("/proc/{pid}")).exists()
    }

    #[cfg(not(target_os = "linux"))]
    fn process_is_absent(pid: u32) -> bool {
        let status = Command::new("kill").args(["-0", &pid.to_string()]).status();
        status.is_ok_and(|status| !status.success())
    }
}
