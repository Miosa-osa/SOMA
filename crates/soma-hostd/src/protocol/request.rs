//! Client request frames.
//!
//! Two families share one socket. Claim, Release, Inspect, and Reconcile address workers and
//! are the allocator's own contract. Launch, Get, List, and Destroy address Instances and are
//! the lifecycle contract of ADR 0031: they name an Instance, never a worker, so a client can
//! address a Machine it did not create and could not name a host resource if it tried.

use soma_netd::NetworkIntent;

use super::{CLAIM_HEADER, MAX_FRAME, ProtocolError, array};
use crate::{InstanceId, LaunchMaterialHandle, OperationId, WorkerId};

/// Everything one Launch or Claim binds to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchFrame {
    /// The operation.
    pub operation: OperationId,
    /// The Instance.
    pub instance: InstanceId,
    /// The vsock CID.
    pub vsock_cid: u32,
    /// Nanoseconds the Instance may live.
    pub deadline_nanos: u64,
    /// The sealed launch material.
    pub launch_material: LaunchMaterialHandle,
    /// The admitted network intent.
    pub intent: NetworkIntent,
}

impl LaunchFrame {
    fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.operation.as_bytes());
        out.extend_from_slice(self.instance.as_bytes());
        out.extend_from_slice(&self.vsock_cid.to_be_bytes());
        out.extend_from_slice(&self.deadline_nanos.to_be_bytes());
        out.extend_from_slice(self.launch_material.as_bytes());
        out.extend_from_slice(&self.intent.encode());
    }

    fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() <= CLAIM_HEADER {
            return Err(ProtocolError("request"));
        }
        Ok(Self {
            operation: OperationId::new(array(&bytes[1..17]))
                .map_err(|_| ProtocolError("operation"))?,
            instance: InstanceId::new(array(&bytes[17..33]))
                .map_err(|_| ProtocolError("instance"))?,
            vsock_cid: u32::from_be_bytes(array(&bytes[33..37])),
            deadline_nanos: u64::from_be_bytes(array(&bytes[37..45])),
            launch_material: LaunchMaterialHandle::new(array(&bytes[45..77]))
                .map_err(|_| ProtocolError("launch material"))?,
            intent: NetworkIntent::decode(&bytes[CLAIM_HEADER..])
                .map_err(|_| ProtocolError("intent"))?,
        })
    }
}

/// One client request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    /// Claim one sterile worker and transfer fresh authority for the Instance.
    Claim(LaunchFrame),
    /// Release one assigned or running worker.
    Release {
        /// The worker.
        worker: WorkerId,
    },
    /// Inspect one worker.
    Inspect {
        /// The worker.
        worker: WorkerId,
    },
    /// Reconcile the ledger.
    Reconcile,
    /// Launch one Instance the Host Runtime owns until a terminal operation.
    Launch(LaunchFrame),
    /// Look one live Instance up by its exact identity.
    Get {
        /// The Instance.
        instance: InstanceId,
    },
    /// List one bounded page of live Instances after `after` in identity order.
    List {
        /// The last identity of the previous page, or `None` for the first page.
        after: Option<InstanceId>,
    },
    /// Destroy one Instance; the operation is idempotent.
    Destroy {
        /// The Instance.
        instance: InstanceId,
    },
}

impl Request {
    /// Encodes the request.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAX_FRAME);
        match self {
            Self::Claim(frame) => {
                out.push(1);
                frame.encode_into(&mut out);
            }
            Self::Release { worker } => {
                out.push(2);
                out.extend_from_slice(worker.as_bytes());
            }
            Self::Inspect { worker } => {
                out.push(3);
                out.extend_from_slice(worker.as_bytes());
            }
            Self::Reconcile => out.push(4),
            Self::Launch(frame) => {
                out.push(5);
                frame.encode_into(&mut out);
            }
            Self::Get { instance } => {
                out.push(6);
                out.extend_from_slice(instance.as_bytes());
            }
            Self::List { after } => {
                out.push(7);
                match after {
                    Some(instance) => {
                        out.push(1);
                        out.extend_from_slice(instance.as_bytes());
                    }
                    None => out.push(0),
                }
            }
            Self::Destroy { instance } => {
                out.push(8);
                out.extend_from_slice(instance.as_bytes());
            }
        }
        out
    }

    /// Decodes one exact request frame.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] for any malformed frame.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.is_empty() || bytes.len() > MAX_FRAME {
            return Err(ProtocolError("frame length"));
        }
        let worker = || WorkerId::new(array(&bytes[1..17])).map_err(|_| ProtocolError("worker"));
        let instance =
            || InstanceId::new(array(&bytes[1..17])).map_err(|_| ProtocolError("instance"));
        match (bytes[0], bytes.len()) {
            (1, _) => LaunchFrame::decode(bytes).map(Self::Claim),
            (2, 17) => Ok(Self::Release { worker: worker()? }),
            (3, 17) => Ok(Self::Inspect { worker: worker()? }),
            (4, 1) => Ok(Self::Reconcile),
            (5, _) => LaunchFrame::decode(bytes).map(Self::Launch),
            (6, 17) => Ok(Self::Get {
                instance: instance()?,
            }),
            (7, 2) if bytes[1] == 0 => Ok(Self::List { after: None }),
            (7, 18) if bytes[1] == 1 => Ok(Self::List {
                after: Some(
                    InstanceId::new(array(&bytes[2..18])).map_err(|_| ProtocolError("instance"))?,
                ),
            }),
            (8, 17) => Ok(Self::Destroy {
                instance: instance()?,
            }),
            _ => Err(ProtocolError("request")),
        }
    }
}
