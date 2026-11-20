//! Whether one command may be sent to this guest at all.
//!
//! A command whose arguments every guest decodes is sent without a word. One with an argument
//! longer than [`crate::LEVEL_TWO_ARGUMENT_BYTES`] ends the session of a guest below API level 3
//! on arrival, so the level is learned first, once per session, with the same probe and cache the
//! level 2 file operations use (ADR 0051), and a guest that lacks it is never sent the command.

use crate::GuestCommand;

use super::super::error::ControlError;
use super::{GuestApi, HostControlIo, RepairedHostControl};

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Returns whether this guest decodes `command`, probing its level first when it must.
    ///
    /// `false` means nothing was sent and the session is exactly as it was: the caller refuses the
    /// command, and the guest goes on answering every command it does decode.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once, when the probe
    /// itself fails.
    pub fn admits(
        self,
        command: &GuestCommand,
        cache: &mut Option<GuestApi>,
        nonce: impl FnOnce() -> Vec<u8>,
    ) -> Result<(Self, bool), ControlError> {
        if !command.needs_long_arguments() {
            return Ok((self, true));
        }
        let (session, level) = self.ensure_guest_api(cache, nonce)?;
        Ok((session, level.has_long_arguments()))
    }
}
