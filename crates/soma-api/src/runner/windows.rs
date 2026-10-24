//! The egress windows of the sandboxes this runner created windowed.
//!
//! A windowed sandbox is created with its egress denied and a broker bundle held ready, so a
//! caller can open the public internet for one call and close it again. This is the runner's
//! record of which sandboxes are windowed and which of them has a window open and until when. It
//! is a record of what the runner asked for, not proof of what is in force: the broker's reply to
//! each change is, and a window recorded here is closed when it runs out whether or not the
//! caller comes back.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use super::ids::SandboxId;

/// The longest a window may stay open. A caller that needs longer opens another.
pub const MAX_WINDOW: Duration = Duration::from_mins(15);
/// The longest `window_id` a caller may name.
pub const MAX_WINDOW_ID_BYTES: usize = 64;

/// One open window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Window {
    /// The caller's name for it; the same id asked twice is the same window.
    pub id: String,
    /// Whether the resolvers are reachable while it is open.
    pub dns: bool,
    /// When the runner closes it if the caller has not.
    pub expires: Instant,
    /// The open has been asked of the broker and not yet answered.
    pub opening: bool,
}

/// What asking to open a window found.
#[derive(Debug, Eq, PartialEq)]
pub enum Opening {
    /// No window was open; this one is now recorded as opening.
    Fresh,
    /// This window is already open: the same request answers the same way and extends nothing.
    Already(Window),
    /// The same window is still being opened by another call.
    InFlight,
    /// A different window is open.
    Conflict(Window),
}

#[derive(Debug, Default)]
struct Entry {
    /// What the create named: `denied`, `internet` or `unrestricted`.
    base: Option<&'static str>,
    windowed: bool,
    window: Option<Window>,
}

/// The windows of one runner.
#[derive(Debug, Default)]
pub struct Windows {
    inner: Mutex<HashMap<SandboxId, Entry>>,
}

impl Windows {
    fn lock(&self) -> MutexGuard<'_, HashMap<SandboxId, Entry>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records the network a sandbox was created with, when the create named one.
    pub fn created(&self, id: &SandboxId, base: &'static str, windowed: bool) {
        self.lock().insert(
            id.clone(),
            Entry {
                base: Some(base),
                windowed,
                window: None,
            },
        );
    }

    /// Whether the runner knows this sandbox as windowed.
    #[must_use]
    pub fn is_windowed(&self, id: &SandboxId) -> bool {
        self.lock().get(id).is_some_and(|entry| entry.windowed)
    }

    /// What the create named, when it named one.
    #[must_use]
    pub fn base(&self, id: &SandboxId) -> Option<&'static str> {
        self.lock().get(id).and_then(|entry| entry.base)
    }

    /// Marks a window as due now, so the next sweep closes it again. Used when a close was
    /// asked and not confirmed: the window stays recorded until the broker says it is shut.
    pub fn expire_now(&self, id: &SandboxId) {
        if let Some(window) = self
            .lock()
            .get_mut(id)
            .and_then(|entry| entry.window.as_mut())
        {
            window.expires = Instant::now();
            window.opening = false;
        }
    }

    /// The window open on a sandbox, if any.
    #[must_use]
    pub fn current(&self, id: &SandboxId) -> Option<Window> {
        self.lock().get(id).and_then(|entry| entry.window.clone())
    }

    /// Records a window as opening, unless one is already there.
    ///
    /// The window is recorded before the broker is asked, so a runner that dies or loses the
    /// answer still has it to close.
    pub fn begin_open(&self, id: &SandboxId, wanted: Window) -> Opening {
        let mut windows = self.lock();
        let entry = windows.entry(id.clone()).or_default();
        match &entry.window {
            Some(open) if open.id == wanted.id && open.opening => Opening::InFlight,
            Some(open) if open.id == wanted.id => Opening::Already(open.clone()),
            Some(open) => Opening::Conflict(open.clone()),
            None => {
                entry.window = Some(Window {
                    opening: true,
                    ..wanted
                });
                Opening::Fresh
            }
        }
    }

    /// The broker confirmed the window; it is open until it expires.
    pub fn opened(&self, id: &SandboxId, window_id: &str) {
        if let Some(window) = self
            .lock()
            .get_mut(id)
            .and_then(|entry| entry.window.as_mut())
            .filter(|window| window.id == window_id)
        {
            window.opening = false;
        }
    }

    /// Forgets the window on a sandbox, whatever its id: egress is denied again.
    pub fn closed(&self, id: &SandboxId) {
        if let Some(entry) = self.lock().get_mut(id) {
            entry.window = None;
        }
    }

    /// Forgets a sandbox that no longer exists.
    pub fn forget(&self, id: &SandboxId) {
        self.lock().remove(id);
    }

    /// The sandboxes whose window has run out (or never finished opening), to be closed.
    #[must_use]
    pub fn expired(&self, now: Instant) -> Vec<SandboxId> {
        self.lock()
            .iter()
            .filter(|(_, entry)| {
                entry
                    .window
                    .as_ref()
                    .is_some_and(|window| window.expires <= now)
            })
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Whether `id` is a name a caller may give a window: short, printable, and free of anything a
/// log line or a path would have to escape.
#[must_use]
pub fn valid_window_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_WINDOW_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> SandboxId {
        SandboxId::mint('a')
    }

    fn window(id: &str, lasting: Duration) -> Window {
        Window {
            id: id.to_owned(),
            dns: true,
            expires: Instant::now() + lasting,
            opening: false,
        }
    }

    #[test]
    fn a_window_is_opened_once_and_the_same_id_is_the_same_window() {
        let windows = Windows::default();
        let id = sandbox();
        windows.created(&id, "denied", true);

        assert_eq!(
            windows.begin_open(&id, window("w1", Duration::from_secs(30))),
            Opening::Fresh
        );
        // A second call while the first is still being opened must not be answered as done.
        assert_eq!(
            windows.begin_open(&id, window("w1", Duration::from_secs(30))),
            Opening::InFlight
        );
        windows.opened(&id, "w1");
        let Opening::Already(open) = windows.begin_open(&id, window("w1", Duration::from_secs(99)))
        else {
            panic!("the same id is the same window");
        };
        // And it extends nothing: the second request's longer ttl is ignored.
        assert!(open.expires < Instant::now() + Duration::from_secs(40));
    }

    #[test]
    fn a_second_window_is_refused_while_one_is_open() {
        let windows = Windows::default();
        let id = sandbox();
        windows.created(&id, "denied", true);
        assert_eq!(
            windows.begin_open(&id, window("w1", Duration::from_secs(30))),
            Opening::Fresh
        );
        windows.opened(&id, "w1");

        assert!(matches!(
            windows.begin_open(&id, window("w2", Duration::from_secs(30))),
            Opening::Conflict(open) if open.id == "w1"
        ));
        windows.closed(&id);
        assert_eq!(
            windows.begin_open(&id, window("w2", Duration::from_secs(30))),
            Opening::Fresh
        );
    }

    #[test]
    fn an_expired_window_is_found_for_closing_and_a_live_one_is_not() {
        let windows = Windows::default();
        let (gone, live, never_opened) = (sandbox(), sandbox(), sandbox());
        for id in [&gone, &live, &never_opened] {
            windows.created(id, "denied", true);
        }
        windows.begin_open(&gone, window("w", Duration::ZERO));
        windows.begin_open(&live, window("w", Duration::from_mins(5)));
        let now = Instant::now() + Duration::from_millis(1);

        assert_eq!(windows.expired(now), vec![gone.clone()]);
        // A window that never finished opening is closed the same way once it runs out.
        windows.opened(&gone, "w");
        windows.closed(&gone);
        assert!(windows.expired(now).is_empty());
    }

    #[test]
    fn only_sandboxes_created_windowed_are_known_as_windowed() {
        let windows = Windows::default();
        let (windowed, plain, plain_named) = (sandbox(), sandbox(), sandbox());
        windows.created(&windowed, "denied", true);
        windows.created(&plain_named, "denied", false);

        assert!(windows.is_windowed(&windowed));
        assert!(!windows.is_windowed(&plain));
        assert!(
            !windows.is_windowed(&plain_named),
            "a named denied sandbox is not windowed"
        );
        assert_eq!(windows.base(&plain_named), Some("denied"));
        assert_eq!(windows.base(&plain), None);
        windows.forget(&windowed);
        assert!(!windows.is_windowed(&windowed));
    }

    #[test]
    fn a_window_id_is_short_and_plain() {
        assert!(valid_window_id("0b9f2a4e-1c3d-4e5f-8a7b-9c0d1e2f3a4b"));
        assert!(valid_window_id("call_17.retry-2"));
        for bad in ["", "has space", "a/b", "line\nbreak", "ü", &"x".repeat(65)] {
            assert!(!valid_window_id(bad), "{bad:?}");
        }
    }
}
