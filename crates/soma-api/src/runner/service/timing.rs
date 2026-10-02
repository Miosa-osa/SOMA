//! The `Server-Timing` header every runner answer carries (contract C2).

use std::{fmt::Write as _, time::Duration};

use crate::runner::backend::CallTiming;

/// Where each request's time went, as `Server-Timing` reports it in milliseconds.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Timing {
    pub(super) auth: Duration,
    pub(super) call: CallTiming,
}

impl Timing {
    pub(super) fn header(self) -> String {
        let mut value = String::with_capacity(48);
        for (index, (name, duration)) in [
            ("auth", self.auth),
            ("pool", self.call.pool),
            ("exec", self.call.exec),
        ]
        .into_iter()
        .enumerate()
        {
            if index > 0 {
                value.push(',');
            }
            let micros = duration.as_micros();
            let _written = write!(value, "{name};dur={}.{:03}", micros / 1_000, micros % 1_000);
        }
        value
    }
}
