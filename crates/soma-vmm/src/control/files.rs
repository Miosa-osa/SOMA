//! Bounded binary-safe filesystem messages on a private socket lent for one request.

use std::io::{self, Read, Write};

use soma::{FileAnswer, FileOperation, MAX_FILE_BYTES};

// JSON represents each byte as up to three digits and a comma. Leave room for the
// operation, paths, and directory entry metadata, and require the bound before reading.
const MAX_FRAME: usize = 4 * MAX_FILE_BYTES + 256 * 1024;

pub const FILE_CONTROL_CEILING: std::time::Duration = std::time::Duration::from_secs(125);

/// Sends a checked operation on the socket attached to the file control request.
///
/// # Errors
/// Returns the socket or framing error when the whole request could not be sent.
pub fn write_file_request(socket: &mut impl Write, operation: &FileOperation) -> io::Result<()> {
    write_frame(
        socket,
        &serde_json::to_vec(operation).map_err(io::Error::other)?,
    )
}

/// Receives an operation from the supervisor without buffering beyond the frame bound.
///
/// # Errors
/// Returns the socket, framing, or decoding error when the operation is not complete.
pub fn read_file_request(socket: &mut impl Read) -> io::Result<FileOperation> {
    serde_json::from_slice(&read_frame(socket)?).map_err(io::Error::other)
}

/// Sends the guest's typed answer, including any bytes, on the same private socket.
///
/// # Errors
/// Returns the socket or framing error when the whole answer could not be sent.
pub fn write_file_answer(socket: &mut impl Write, answer: &FileAnswer) -> io::Result<()> {
    write_frame(
        socket,
        &serde_json::to_vec(answer).map_err(io::Error::other)?,
    )
}

/// Receives the guest's typed answer after one file operation.
///
/// # Errors
/// Returns the socket, framing, or decoding error when the answer is not complete.
pub fn read_file_answer(socket: &mut impl Read) -> io::Result<FileAnswer> {
    serde_json::from_slice(&read_frame(socket)?).map_err(io::Error::other)
}

fn write_frame(socket: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    let length = frame_length(frame.len())?;
    socket.write_all(&length.to_be_bytes())?;
    socket.write_all(frame)
}

fn read_frame(socket: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut prefix = [0_u8; 4];
    socket.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    frame_length(length)?;
    let mut frame = vec![0_u8; length];
    socket.read_exact(&mut frame)?;
    Ok(frame)
}

fn frame_length(length: usize) -> io::Result<u32> {
    if length == 0 || length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file frame length",
        ));
    }
    u32::try_from(length).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_binary_answer_round_trips() {
        let bytes = vec![0, 127, 128, 255].repeat(128 * 1024);
        let mut wire = Vec::new();
        write_file_answer(
            &mut wire,
            &FileAnswer::Read {
                bytes: bytes.clone(),
            },
        )
        .expect("encoded answer");
        assert_eq!(
            read_file_answer(&mut wire.as_slice()).expect("decoded answer"),
            FileAnswer::Read { bytes }
        );
    }

    #[test]
    fn rejects_overlong_and_truncated_frames() {
        let prefix = u32::MAX.to_be_bytes();
        assert_eq!(
            read_file_request(&mut prefix.as_slice())
                .expect_err("length")
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            read_file_request(&mut [0, 0, 0, 3, b'{'].as_slice())
                .expect_err("short")
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
