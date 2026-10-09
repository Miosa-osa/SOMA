//! What one failed execute operation leaves behind.
//!
//! An execute that fails has two possible consequences and they are not the same one: the
//! sandbox is finished, or the sandbox is exactly where it was. Reading every failure as the
//! first is what let a tenant lose a sandbox by asking for something as ordinary as a program
//! that does not exist, so the question is asked explicitly here rather than assumed by the
//! failure path.
//!
//! The line runs between outcomes somebody observed and outcomes nobody did. The guest agent
//! answers one command at a time over the authenticated session, and it kills and reaps each
//! command's whole process group before it reports the command's status, so a status from the
//! agent is proof that the agent is alive and holds nothing of that command. A refusal means the
//! command never reached the machine at all. Neither leaves work behind. Everything else - a
//! dead session, a wedged agent, evidence that did not survive validation - leaves a machine
//! whose state is unknown, and an unknown machine is released.

/// What one failed execute operation leaves behind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommandDisposition {
    /// The backend would not carry the command as far as the machine.
    ///
    /// The command violated a bound the runtime applies before it runs anything, so nothing was
    /// created and nothing was started. The machine was never addressed.
    RefusedBeforeRun,
    /// The guest agent could not start the program, so no process ever existed.
    ///
    /// The agent is running and answered; the invocation alone was refused. This is what an
    /// absolute path that does not exist produces.
    SpawnRefused,
    /// The guest agent ran the command and stopped it, proving its process group gone.
    ///
    /// A deadline that expired and an output allowance that ran out both take this shape: the
    /// agent kills the group, reaps every member of it, sweeps the machine for strays, and only
    /// then reports the terminal status. The agent is therefore healthy and idle.
    StoppedByTheGuest,
    /// Nobody observed what the command did, or whether the machine is still there.
    Unobserved,
}

impl CommandDisposition {
    /// Whether the machine that produced this outcome may keep running.
    ///
    /// Only an outcome nobody observed releases the machine. Releasing one for a request that
    /// merely asked for something the machine would not do is how a caller lost a sandbox it
    /// had done nothing to lose.
    pub(super) const fn keeps_the_machine(self) -> bool {
        match self {
            Self::RefusedBeforeRun | Self::SpawnRefused | Self::StoppedByTheGuest => true,
            Self::Unobserved => false,
        }
    }
}
