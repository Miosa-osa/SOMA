//! One bounded, process-group-contained invocation of one external tool.
//!
//! Every SOMA component that runs a host tool runs it through this module: the Generation
//! compiler's pinned build tools and the privileged network broker's `nft` and `conntrack`.
//! The contract each caller gets is the same one: the tool leads its own process group, so a
//! deadline, a feed failure, a capture overflow, or a cancellation terminates the tool and
//! every descendant it forked rather than only the direct child; termination, draining,
//! waiting, and collection are each separately bounded; and the retained output is bounded
//! before it is allocated.
//!
//! A caller that cannot reach a trustworthy result gets [`Uncontained`] naming which bound was
//! reached, never a partial success.

mod capture;
mod contained;
mod group;
#[cfg(target_os = "linux")]
mod handoff;
mod supervise;

pub use capture::CAPTURE_LIMIT;
pub use contained::{Contained, Output, TERMINATION_GRACE, Uncontained};
#[cfg(target_os = "linux")]
pub use handoff::{DescriptorHandoffError, receive_descriptors, send_descriptors};
