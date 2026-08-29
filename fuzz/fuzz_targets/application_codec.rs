#![no_main]

use std::sync::{Once, OnceLock};

use libfuzzer_sys::fuzz_target;
use soma_guest::{
    GuestCommand, GuestMessage, HostMessage, MAX_RECORD_PAYLOAD, OperationId, OutputChunk,
    TerminalReport, TerminalStatus,
};

const HEADER_SIZE: usize = 28;
static EXERCISE_CANONICAL_ONCE: Once = Once::new();
static CANONICAL: OnceLock<Vec<Vec<u8>>> = OnceLock::new();

fuzz_target!(|input: &[u8]| {
    exercise(input);

    let canonical = canonical_messages();
    EXERCISE_CANONICAL_ONCE.call_once(|| {
        for message in canonical {
            exercise(message);
        }
    });

    let selector = usize::from(input.first().copied().unwrap_or(0)) % canonical.len();
    exercise(&deep_mutation(&canonical[selector], input));
});

fn exercise(input: &[u8]) {
    if let Ok(message) = HostMessage::decode(input) {
        let canonical = message.encode().expect("accepted host message must encode");
        assert_eq!(canonical, input);
        assert_eq!(HostMessage::decode(&canonical), Ok(message));
    }
    if let Ok(message) = GuestMessage::decode(input) {
        let canonical = message
            .encode()
            .expect("accepted guest message must encode");
        assert_eq!(canonical, input);
        assert_eq!(GuestMessage::decode(&canonical), Ok(message));
    }
}

fn canonical_messages() -> &'static [Vec<u8>] {
    CANONICAL.get_or_init(|| {
        let operation = OperationId::new([7; 16]).expect("operation");
        let command = GuestCommand::new(
            b"/usr/bin/printf".to_vec(),
            vec![b"%s".to_vec(), b"deep".to_vec()],
            1_000,
            4_096,
        )
        .expect("command");
        let maximum = maximum_command(operation);
        vec![
            HostMessage::prepare_and_probe(operation)
                .encode()
                .expect("PrepareAndProbe"),
            HostMessage::execute(operation, command)
                .encode()
                .expect("Execute"),
            HostMessage::shutdown(operation).encode().expect("Shutdown"),
            GuestMessage::repair_complete(operation)
                .encode()
                .expect("RepairComplete"),
            GuestMessage::stdout(
                operation,
                OutputChunk::new(vec![0xA5; 4_096]).expect("maximum output chunk"),
            )
            .encode()
            .expect("Stdout"),
            GuestMessage::stderr(
                operation,
                OutputChunk::new(b"binary\0stderr".to_vec()).expect("stderr chunk"),
            )
            .encode()
            .expect("Stderr"),
            GuestMessage::terminal(
                operation,
                TerminalReport::new(TerminalStatus::OutputLimit, 16 * 1024 * 1024, 0)
                    .expect("terminal report"),
            )
            .encode()
            .expect("Terminal"),
            GuestMessage::shutdown_ack(operation)
                .encode()
                .expect("ShutdownAck"),
            maximum,
        ]
    })
}

fn maximum_command(operation: OperationId) -> Vec<u8> {
    let mut arguments = vec![vec![b'x'; 4_096]; 15];
    arguments.push(vec![b'y'; 3_991]);
    let encoded = HostMessage::execute(
        operation,
        GuestCommand::new(b"/p".to_vec(), arguments, 1, 1).expect("maximum command"),
    )
    .encode()
    .expect("maximum Execute");
    assert_eq!(encoded.len(), MAX_RECORD_PAYLOAD);
    encoded
}

fn deep_mutation(seed: &[u8], input: &[u8]) -> Vec<u8> {
    let mut mutated = seed.to_vec();
    let start = if mutated.len() > HEADER_SIZE {
        HEADER_SIZE
    } else {
        10
    };
    let span = mutated.len() - start;
    let mutation_count = input.len().min(256);
    for (ordinal, byte) in input.iter().take(mutation_count).enumerate() {
        let distributed = ordinal
            .checked_mul(span)
            .expect("canonical message span fits usize")
            / mutation_count;
        let offset = distributed.wrapping_add(usize::from(*byte)) % span;
        mutated[start + offset] ^= *byte;
    }
    mutated
}
