//! Typestated repair controller that owns every lifecycle transition of the guest agent.
//!
//! The compile-time state markers make an out-of-order or duplicate transition unrepresentable
//! through the safe API, and the runtime ledger re-checks the same order so a future refactor
//! that bypasses the markers still fails closed.
//! Every failure consumes the controller into the terminal `Poisoned` state.

use core::marker::PhantomData;

/// Redacted reason for poisoning the single-use guest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fault {
    /// Early init could not compose the root or find the expected devices.
    Boot,
    /// The launch page was absent, malformed, or could not be erased.
    LaunchPage,
    /// Fresh entropy could not be mixed into the kernel or verified.
    Entropy,
    /// The fixed vsock control transport could not be opened fresh.
    Transport,
    /// Hostname, machine identity, session directories, or wall clock could not be replaced.
    Identity,
    /// The network identity could not be installed.
    Network,
    /// The authenticated control lifecycle failed or the peer violated it.
    Control,
    /// A lifecycle transition was attempted out of the fixed order.
    Order,
}

/// Observable lifecycle phase for diagnostics and tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Restored from the disconnected repair point with no fresh authority.
    Captured,
    /// The launch page was consumed and erased.
    MaterialAccepted,
    /// The kernel CSPRNG was reseeded from fresh host entropy.
    EntropyRepaired,
    /// A fresh vsock control transport exists.
    TransportFresh,
    /// Hostname, machine identity, session state, and wall clock were replaced.
    IdentityRepaired,
    /// The fresh network identity is installed.
    NetworkRepaired,
    /// The Noise session with the host is authenticated.
    Authenticated,
    /// Repair was committed and reported under the authenticated session.
    Prepared,
    /// Authenticated commands are accepted.
    Ready,
    /// One command is in flight.
    Running,
    /// An authenticated shutdown is in progress and no new work is accepted.
    Stopping,
    /// The single-use guest is destroyed.
    Poisoned,
}

impl Phase {
    /// Returns whether `next` is a legal successor of this phase.
    #[must_use]
    pub const fn permits(self, next: Self) -> bool {
        if matches!(next, Self::Poisoned) {
            return !matches!(self, Self::Poisoned);
        }
        matches!(
            (self, next),
            (Self::Captured, Self::MaterialAccepted)
                | (Self::MaterialAccepted, Self::EntropyRepaired)
                | (Self::EntropyRepaired, Self::TransportFresh)
                | (Self::TransportFresh, Self::IdentityRepaired)
                | (Self::IdentityRepaired, Self::NetworkRepaired)
                | (Self::NetworkRepaired, Self::Authenticated)
                | (Self::Authenticated, Self::Prepared)
                | (Self::Prepared | Self::Running, Self::Ready)
                | (Self::Ready, Self::Running | Self::Stopping)
        )
    }
}

/// Runtime record of the transitions taken by one controller.
#[derive(Debug)]
pub struct Ledger {
    current: Phase,
    repaired: Vec<Phase>,
    commands: u64,
    fault: Option<Fault>,
}

impl Ledger {
    fn new() -> Self {
        Self {
            current: Phase::Captured,
            repaired: vec![Phase::Captured],
            commands: 0,
            fault: None,
        }
    }

    fn advance(&mut self, next: Phase) -> Result<(), Fault> {
        if !self.current.permits(next) {
            return Err(Fault::Order);
        }
        match (self.current, next) {
            (Phase::Ready, Phase::Running) => {
                self.commands = self.commands.checked_add(1).ok_or(Fault::Order)?;
            }
            (Phase::Running, Phase::Ready) => {}
            _ => self.repaired.push(next),
        }
        self.current = next;
        Ok(())
    }

    /// Returns the current phase.
    #[cfg(test)]
    pub const fn current(&self) -> Phase {
        self.current
    }

    /// Returns the ordered repair chain recorded so far, excluding Ready and Running cycles.
    #[must_use]
    pub fn repair_chain(&self) -> &[Phase] {
        &self.repaired
    }

    /// Returns the number of commands admitted after Ready.
    #[cfg(test)]
    pub const fn commands(&self) -> u64 {
        self.commands
    }

    /// Returns the poisoning fault, if any.
    #[must_use]
    pub const fn fault(&self) -> Option<Fault> {
        self.fault
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Compile-time lifecycle marker.
pub trait State: sealed::Sealed {
    /// The phase this marker represents.
    const PHASE: Phase;
}

macro_rules! states {
    ($($name:ident => $phase:ident),* $(,)?) => {
        $(
            #[doc = concat!("Marker for the `", stringify!($phase), "` phase.")]
            #[derive(Debug)]
            pub struct $name;
            impl sealed::Sealed for $name {}
            impl State for $name {
                const PHASE: Phase = Phase::$phase;
            }
        )*
    };
}

states! {
    Captured => Captured,
    MaterialAccepted => MaterialAccepted,
    EntropyRepaired => EntropyRepaired,
    TransportFresh => TransportFresh,
    IdentityRepaired => IdentityRepaired,
    NetworkRepaired => NetworkRepaired,
    Authenticated => Authenticated,
    Prepared => Prepared,
    Ready => Ready,
    Running => Running,
    Stopping => Stopping,
    Poisoned => Poisoned,
}

/// The single owner of the guest lifecycle.
#[derive(Debug)]
pub struct Controller<S: State> {
    ledger: Ledger,
    _state: PhantomData<S>,
}

/// Result of one transition: the next owner with its evidence, or the poisoned owner.
pub type Step<S, T> = Result<(Controller<S>, T), Controller<Poisoned>>;

impl<S: State> Controller<S> {
    /// Returns the runtime ledger for diagnostics and tests.
    #[must_use]
    pub const fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Consumes this owner into the terminal poisoned state.
    #[must_use]
    pub fn poison(self, fault: Fault) -> Controller<Poisoned> {
        poisoned(self.ledger, fault)
    }

    fn step<N: State, T>(self, outcome: Result<T, Fault>) -> Step<N, T> {
        let mut ledger = self.ledger;
        match outcome.and_then(|value| ledger.advance(N::PHASE).map(|()| value)) {
            Ok(value) => Ok((
                Controller {
                    ledger,
                    _state: PhantomData,
                },
                value,
            )),
            Err(fault) => Err(poisoned(ledger, fault)),
        }
    }
}

fn poisoned(mut ledger: Ledger, fault: Fault) -> Controller<Poisoned> {
    ledger.fault = Some(fault);
    ledger.current = Phase::Poisoned;
    Controller {
        ledger,
        _state: PhantomData,
    }
}

macro_rules! transition {
    ($from:ident, $method:ident, $to:ident) => {
        impl Controller<$from> {
            #[doc = concat!("Advances to `", stringify!($to), "` with the outcome of that stage.")]
            pub fn $method<T>(self, outcome: Result<T, Fault>) -> Step<$to, T> {
                self.step(outcome)
            }
        }
    };
}

impl Controller<Captured> {
    /// Creates the controller at the disconnected repair point.
    #[must_use]
    pub fn captured() -> Self {
        Self {
            ledger: Ledger::new(),
            _state: PhantomData,
        }
    }
}

impl Default for Controller<Captured> {
    fn default() -> Self {
        Self::captured()
    }
}

transition!(Captured, accept_material, MaterialAccepted);
transition!(MaterialAccepted, repair_entropy, EntropyRepaired);
transition!(EntropyRepaired, freshen_transport, TransportFresh);
transition!(TransportFresh, repair_identity, IdentityRepaired);
transition!(IdentityRepaired, repair_network, NetworkRepaired);
transition!(NetworkRepaired, authenticate, Authenticated);
transition!(Authenticated, prepare, Prepared);
transition!(Prepared, ready, Ready);
transition!(Ready, run, Running);
transition!(Running, finish, Ready);
transition!(Ready, stop, Stopping);

impl Controller<Poisoned> {
    /// Returns the fault that destroyed this guest.
    #[must_use]
    pub fn fault(&self) -> Fault {
        self.ledger.fault().unwrap_or(Fault::Order)
    }
}

#[cfg(test)]
mod tests;
