mod control_support;

use core::convert::Infallible;
use std::time::{Duration, Instant};

use soma_guest::{
    ControlIo, GuestCommand, GuestControl, GuestLaunchMaterial, GuestRequest, HostControl,
    HostControlIo, HostLaunchMaterial, HostMessage, LAUNCH_PAGE_SIZE, OperationId, TerminalStatus,
};

use control_support::{deadline, launch_network, pair};

fn accepts_control_io<T: ControlIo>() {}
fn accepts_host_control_io<T: HostControlIo>() {}

#[test]
fn control_io_interfaces_are_public() {
    let _ = accepts_control_io::<NeverIo>;
    let _ = accepts_host_control_io::<NeverIo>;
}

#[test]
fn readiness_message_has_no_caller_supplied_command() {
    let operation = OperationId::new([7; 16]).expect("operation");
    let message = HostMessage::prepare_and_probe(operation);
    let encoded = message.encode().expect("fixed readiness probe");

    assert_eq!(HostMessage::decode(&encoded), Ok(message));
}

#[test]
fn owners_complete_both_handshake_messages_before_connecting() {
    let host = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], launch_network())
        .expect("host launch material");
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    let host = host
        .deliver_with(|bytes| {
            page.copy_from_slice(bytes);
            Ok::<(), Infallible>(())
        })
        .expect("page delivery");
    let guest = GuestLaunchMaterial::take_from_page(&mut page)
        .expect("guest launch material")
        .reseed_with(|_| Ok::<(), Infallible>(()))
        .expect("guest entropy repair");
    let (host_io, guest_io, host_observed, guest_observed) = pair();

    let guest_thread = std::thread::spawn(move || {
        GuestControl::connect(guest, guest_io, deadline()).expect("guest owner connected")
    });
    let host = HostControl::connect(host, host_io).expect("host owner connected");
    let guest = guest_thread.join().expect("guest thread");

    assert_eq!(host_observed.poison_calls(), 0);
    assert_eq!(host_observed.repair_commits(), 0);
    assert_eq!(guest_observed.poison_calls(), 0);
    drop((host, guest));
}

#[test]
fn one_owner_path_repairs_executes_and_shuts_down() {
    let host = HostLaunchMaterial::generate([4; 32], [5; 16], [6; 16], launch_network())
        .expect("host launch material");
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    let host = host
        .deliver_with(|bytes| {
            page.copy_from_slice(bytes);
            Ok::<(), Infallible>(())
        })
        .expect("page delivery");
    let guest = GuestLaunchMaterial::take_from_page(&mut page)
        .expect("guest launch material")
        .reseed_with(|_| Ok::<(), Infallible>(()))
        .expect("guest entropy repair");
    let execute = OperationId::new([7; 16]).expect("execute operation");
    let shutdown = OperationId::new([8; 16]).expect("shutdown operation");
    let command = GuestCommand::new(b"/bin/echo".to_vec(), vec![b"hello".to_vec()], 100, 8)
        .expect("execute command");
    let expected_command = command.clone();
    let (host_io, guest_io, host_observed, guest_observed) = pair();

    let guest_thread = std::thread::spawn(move || {
        let guest =
            GuestControl::connect(guest, guest_io, deadline()).expect("guest owner connected");
        let (guest, request) = guest.next_request(deadline()).expect("prepare request");
        assert_eq!(
            request,
            GuestRequest::PrepareAndProbe {
                operation: OperationId::new([6; 16]).expect("launch operation")
            }
        );
        let guest = guest.repair_complete(deadline()).expect("repair complete");
        let guest = guest
            .terminal(TerminalStatus::Exited(0), deadline())
            .expect("probe terminal");
        let (guest, request) = guest.next_request(deadline()).expect("execute request");
        assert_eq!(
            request,
            GuestRequest::Execute {
                operation: execute,
                command: expected_command,
            }
        );
        let guest = guest.stdout(b"hello".to_vec(), deadline()).expect("stdout");
        let guest = guest.stderr(b"bad".to_vec(), deadline()).expect("stderr");
        let guest = guest
            .terminal(TerminalStatus::Exited(17), deadline())
            .expect("execute terminal");
        let (guest, request) = guest.next_request(deadline()).expect("shutdown request");
        assert_eq!(
            request,
            GuestRequest::Shutdown {
                operation: shutdown
            }
        );
        guest
            .shutdown_ack(deadline())
            .expect("shutdown acknowledgement");
    });

    let host = HostControl::connect(host, host_io).expect("host owner connected");
    let host = host.prepare_and_probe().expect("authenticated Ready");
    assert_eq!(host_observed.repair_commits(), 1);
    let (host, outcome) = host.execute(execute, command).expect("execute outcome");
    assert_eq!(outcome.status(), TerminalStatus::Exited(17));
    assert_eq!(outcome.stdout(), b"hello");
    assert_eq!(outcome.stderr(), b"bad");
    assert!(!format!("{outcome:?}").contains("hello"));
    host.shutdown(shutdown).expect("graceful shutdown");
    guest_thread.join().expect("guest thread");

    assert_eq!(host_observed.poison_calls(), 0);
    assert_eq!(guest_observed.poison_calls(), 0);

    let host_reads = host_observed.read_deadlines();
    let host_writes = host_observed.write_deadlines();
    let repair_deadlines = host_observed.repair_deadlines();
    assert_frame_deadlines_are_shared(&host_reads);
    assert_eq!(host_writes.len(), 4);
    assert_eq!(repair_deadlines, vec![host_writes[1]]);
    assert_eq!(host_reads[2], host_writes[1]);
    assert!(
        host_reads[6..12]
            .iter()
            .all(|value| *value == host_writes[2])
    );
    assert_eq!(host_reads[12], host_writes[3]);

    let guest_reads = guest_observed.read_deadlines();
    assert_frame_deadlines_are_shared(&guest_reads);
}

fn assert_frame_deadlines_are_shared(deadlines: &[Instant]) {
    let (frames, remainder) = deadlines.as_chunks::<2>();
    assert!(remainder.is_empty());
    assert!(frames.iter().all(|frame| frame[0] == frame[1]));
}

#[test]
fn expired_guest_connect_deadline_fails_closed_without_peer_input() {
    let host = HostLaunchMaterial::generate([31; 32], [32; 16], [33; 16], launch_network())
        .expect("host launch material");
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    let _host = host
        .deliver_with(|bytes| {
            page.copy_from_slice(bytes);
            Ok::<(), Infallible>(())
        })
        .expect("page delivery");
    let guest = GuestLaunchMaterial::take_from_page(&mut page)
        .expect("guest launch material")
        .reseed_with(|_| Ok::<(), Infallible>(()))
        .expect("guest entropy repair");
    let (_host_io, guest_io, _host_observed, guest_observed) = pair();
    let expired = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("expired deadline is representable");

    let result = GuestControl::connect(guest, guest_io, expired);

    assert!(result.is_err());
    assert_eq!(guest_observed.poison_calls(), 1);
    assert_eq!(guest_observed.read_deadlines(), vec![expired]);
}

#[test]
fn every_valid_terminal_outcome_and_exact_output_limit_succeeds() {
    let host = HostLaunchMaterial::generate([9; 32], [10; 16], [11; 16], launch_network())
        .expect("host launch material");
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    let host = host
        .deliver_with(|bytes| {
            page.copy_from_slice(bytes);
            Ok::<(), Infallible>(())
        })
        .expect("page delivery");
    let guest = GuestLaunchMaterial::take_from_page(&mut page)
        .expect("guest launch material")
        .reseed_with(|_| Ok::<(), Infallible>(()))
        .expect("guest entropy repair");
    let statuses = [
        TerminalStatus::Exited(23),
        TerminalStatus::Signaled(9),
        TerminalStatus::TimedOut,
        TerminalStatus::OutputLimit,
        TerminalStatus::ExecFailed(2),
        TerminalStatus::AgentFailed(1),
    ];
    let (host_io, guest_io, host_observed, guest_observed) = pair();

    let guest_thread = std::thread::spawn(move || {
        let guest =
            GuestControl::connect(guest, guest_io, deadline()).expect("guest owner connected");
        let (guest, _) = guest.next_request(deadline()).expect("prepare request");
        let mut guest = guest.repair_complete(deadline()).expect("repair complete");
        guest = guest
            .terminal(TerminalStatus::Exited(0), deadline())
            .expect("probe terminal");
        for status in statuses {
            let (next, request) = guest.next_request(deadline()).expect("execute request");
            assert!(matches!(request, GuestRequest::Execute { .. }));
            guest = if status == TerminalStatus::OutputLimit {
                next.stdout(vec![0xA5], deadline())
                    .expect("exact allowance output")
            } else {
                next
            };
            guest = guest.terminal(status, deadline()).expect("typed terminal");
        }
        let (guest, _) = guest.next_request(deadline()).expect("shutdown request");
        guest
            .shutdown_ack(deadline())
            .expect("shutdown acknowledgement");
    });

    let mut host = HostControl::connect(host, host_io)
        .expect("host owner connected")
        .prepare_and_probe()
        .expect("authenticated Ready");
    for (index, status) in statuses.into_iter().enumerate() {
        let operation = OperationId::new([20 + u8::try_from(index).expect("small index"); 16])
            .expect("operation");
        let command =
            GuestCommand::new(b"/bin/true".to_vec(), vec![], 100, 1).expect("execute command");
        let (next, outcome) = host.execute(operation, command).expect("typed outcome");
        assert_eq!(outcome.status(), status);
        if status == TerminalStatus::OutputLimit {
            assert_eq!(outcome.stdout(), &[0xA5]);
        }
        host = next;
    }
    host.shutdown(OperationId::new([30; 16]).expect("shutdown"))
        .expect("graceful shutdown");
    guest_thread.join().expect("guest thread");
    assert_eq!(host_observed.poison_calls(), 0);
    assert_eq!(guest_observed.poison_calls(), 0);
}

struct NeverIo;

impl ControlIo for NeverIo {
    type Error = ();

    fn read_exact(&mut self, _bytes: &mut [u8], _deadline: Instant) -> Result<(), Self::Error> {
        unreachable!()
    }

    fn write_all(&mut self, _bytes: &[u8], _deadline: Instant) -> Result<(), Self::Error> {
        unreachable!()
    }

    fn poison(&mut self) {}
}

impl HostControlIo for NeverIo {
    fn commit_repair(&mut self, _deadline: Instant) -> Result<(), Self::Error> {
        unreachable!()
    }
}
