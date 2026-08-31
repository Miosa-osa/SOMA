//! The client half of the daemon protocol, so a Machine is addressable from any process.
//!
//! The encoding lives in this crate and so does the client that speaks it: a second copy of
//! the frame layout in an adapter crate would be a second protocol, free to drift from this
//! one, and the drift would show up as a Machine that cannot be addressed rather than as a
//! compile failure.
//!
//! Every call is one request frame and one reply frame on one connection. The connection owns
//! nothing the Host owns, which is the whole point: a client that exits ends its connection
//! and nothing else, and any later client addresses the same Instance by identity alone.

mod error;
mod socket;

pub use error::ClientError;

use std::path::Path;

use error::refused;
use socket::Connection;

use crate::{
    InstanceId, LaunchFrame, LeaseGeneration, Page, Reply, Request, TerminalReceipt, WorkerId,
};

/// What the Host Runtime answered a Launch with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Registration {
    /// The Instance is live and the Host Runtime owns it.
    Live {
        /// The worker serving it.
        worker: WorkerId,
        /// The lease generation the Launch won.
        lease_generation: LeaseGeneration,
        /// CID, generation, MAC, address, prefix, gateway, resolver, time sample.
        launch: [u8; 35],
    },
    /// The operation already holds a worker whose launch page the Host cannot repeat, so this
    /// client destroys the Instance and launches again under a fresh operation.
    Replayed {
        /// The worker.
        worker: WorkerId,
        /// The lease generation.
        lease_generation: LeaseGeneration,
    },
}

/// One live Instance as the Host reports it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveInstance {
    /// The worker serving it.
    pub worker: WorkerId,
    /// The lease generation.
    pub lease_generation: LeaseGeneration,
    /// The phase code of that worker.
    pub phase: u8,
}

/// One connection to a Host daemon.
pub struct HostClient {
    connection: Connection,
}

impl HostClient {
    /// Connects to the Host daemon serving `socket`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Connect`] when nothing is listening there, which is how a caller
    /// learns that this Host has no persistent Runtime rather than that it refused the work.
    pub fn connect(socket: &Path) -> Result<Self, ClientError> {
        Ok(Self {
            connection: Connection::open(socket)?,
        })
    }

    /// Sends one request and returns the reply frame the Host produced, failure included.
    ///
    /// # Errors
    ///
    /// Returns the transport or decoding failure. A refusal by the Host is a successful
    /// exchange and arrives as [`Reply::Failed`]; the typed calls below convert it.
    pub fn call(&self, request: &Request) -> Result<Reply, ClientError> {
        let frame = self.connection.exchange(&request.encode())?;
        Reply::decode(&frame).map_err(|_| ClientError::Protocol)
    }

    /// Launches one Instance the Host Runtime owns until a terminal operation.
    ///
    /// # Errors
    ///
    /// Returns the transport failure, or [`ClientError::Refused`] with the code the Host
    /// answered.
    pub fn launch(&self, frame: &LaunchFrame) -> Result<Registration, ClientError> {
        match self.call(&Request::Launch(frame.clone()))? {
            Reply::Launched {
                worker,
                lease_generation,
                launch,
            } => Ok(Registration::Live {
                worker,
                lease_generation,
                launch,
            }),
            Reply::Replayed {
                worker,
                lease_generation,
            } => Ok(Registration::Replayed {
                worker,
                lease_generation,
            }),
            Reply::Failed(code) => Err(refused(code)),
            _ => Err(ClientError::Unexpected),
        }
    }

    /// Looks one live Instance up by its exact identity.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Refused`] with [`crate::FailureCode::Unknown`] when this Host
    /// owns no such Instance.
    pub fn get(&self, instance: InstanceId) -> Result<LiveInstance, ClientError> {
        match self.call(&Request::Get { instance })? {
            Reply::Live {
                worker,
                lease_generation,
                phase,
            } => Ok(LiveInstance {
                worker,
                lease_generation,
                phase,
            }),
            Reply::Failed(code) => Err(refused(code)),
            _ => Err(ClientError::Unexpected),
        }
    }

    /// Reports one bounded page of live Instances after `after` in identity order.
    ///
    /// # Errors
    ///
    /// Returns the transport failure, or the code the Host answered.
    pub fn list(&self, after: Option<InstanceId>) -> Result<Page, ClientError> {
        match self.call(&Request::List { after })? {
            Reply::Listed { instances, more } => Ok(Page { instances, more }),
            Reply::Failed(code) => Err(refused(code)),
            _ => Err(ClientError::Unexpected),
        }
    }

    /// Destroys one Instance and returns its terminal receipt.
    ///
    /// The call is idempotent at the Host, so a repeat returns the identical receipt rather
    /// than an unknown-Instance refusal that would make a retrying client believe its Machine
    /// is still running.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Refused`] with [`crate::FailureCode::Unknown`] when neither a
    /// live Instance nor a durable record carries the identity.
    pub fn destroy(&self, instance: InstanceId) -> Result<TerminalReceipt, ClientError> {
        match self.call(&Request::Destroy { instance })? {
            Reply::Destroyed { worker, complete } => Ok(TerminalReceipt {
                instance,
                worker,
                complete,
            }),
            Reply::Failed(code) => Err(refused(code)),
            _ => Err(ClientError::Unexpected),
        }
    }
}
