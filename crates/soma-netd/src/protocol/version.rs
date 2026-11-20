/// The version of the broker's frame protocol.
///
/// No frame carries it. A host runs the broker and every client of it from one release, and a
/// frame this build does not know is refused as a protocol error rather than guessed at, so a
/// mixed pair fails closed. The number names the frame layout in `--version` and in the
/// compatibility table of `docs/versioning.md`, and it changes whenever a frame's layout does.
pub const PROTOCOL_VERSION: u16 = 1;
