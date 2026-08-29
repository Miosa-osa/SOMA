//! The topology-aware placement hook: a shape must fit CPU and memory on one node.

use std::fmt;

/// One NUMA node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct NodeId(pub u32);

/// Free capacity of one node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeFree {
    /// The node.
    pub node: NodeId,
    /// Free CPU milli-units.
    pub cpu_milli_units: u64,
    /// Free memory bytes.
    pub memory_bytes: u64,
}

/// What one shape needs on a node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeDemand {
    /// CPU milli-units.
    pub cpu_milli_units: u64,
    /// Memory bytes.
    pub memory_bytes: u64,
}

/// Why no node was chosen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NumaRejection {
    /// The host reported no node.
    NoNodes,
    /// Totals fit but no single node satisfies both dimensions.
    Fragmented {
        /// Nodes examined.
        nodes: u32,
    },
    /// This placement policy handles one node only.
    MultiNodeUnsupported {
        /// Nodes reported.
        nodes: u32,
    },
}

impl fmt::Display for NumaRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoNodes => formatter.write_str("no NUMA node"),
            Self::Fragmented { nodes } => {
                write!(formatter, "no single node of {nodes} fits both dimensions")
            }
            Self::MultiNodeUnsupported { nodes } => {
                write!(
                    formatter,
                    "single-node placement cannot place across {nodes} nodes"
                )
            }
        }
    }
}

impl std::error::Error for NumaRejection {}

/// Chooses the node one shape is placed on.
pub trait NumaPlacement: Send + Sync {
    /// Places `demand` on one node.
    ///
    /// # Errors
    ///
    /// Returns the typed rejection when no node fits both dimensions together.
    fn place(&self, demand: NodeDemand, free: &[NodeFree]) -> Result<NodeId, NumaRejection>;
}

/// The trivial single-node policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SingleNode;

impl NumaPlacement for SingleNode {
    fn place(&self, demand: NodeDemand, free: &[NodeFree]) -> Result<NodeId, NumaRejection> {
        let nodes = u32::try_from(free.len()).unwrap_or(u32::MAX);
        match free {
            [] => Err(NumaRejection::NoNodes),
            [only] => {
                let fits = only.cpu_milli_units >= demand.cpu_milli_units
                    && only.memory_bytes >= demand.memory_bytes;
                fits.then_some(only.node)
                    .ok_or(NumaRejection::Fragmented { nodes: 1 })
            }
            _ => Err(NumaRejection::MultiNodeUnsupported { nodes }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_node_fits_or_names_the_fragmentation() {
        let demand = NodeDemand {
            cpu_milli_units: 8_000,
            memory_bytes: 8 << 30,
        };
        let node = |cpu, memory| NodeFree {
            node: NodeId(0),
            cpu_milli_units: cpu,
            memory_bytes: memory,
        };
        assert_eq!(SingleNode.place(demand, &[]), Err(NumaRejection::NoNodes));
        assert_eq!(
            SingleNode.place(demand, &[node(14_000, 21 << 30)]),
            Ok(NodeId(0))
        );
        assert_eq!(
            SingleNode.place(demand, &[node(2_000, 20 << 30)]),
            Err(NumaRejection::Fragmented { nodes: 1 })
        );
        assert_eq!(
            SingleNode.place(demand, &[node(2_000, 20 << 30), node(12_000, 1 << 30)]),
            Err(NumaRejection::MultiNodeUnsupported { nodes: 2 })
        );
    }
}
