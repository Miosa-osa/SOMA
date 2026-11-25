mod concurrency;
mod durability;
mod enumeration;
mod recovery;
mod release;
#[cfg(unix)]
mod security;
mod turn;

use soma::{InstanceId, StateRecord};

use super::FileStateStore;
use crate::test_support::TempRoot;

const INSTANCE: &str = "0123456789abcdef0123456789abcdef";

fn instance() -> InstanceId {
    InstanceId::new(INSTANCE).expect("fixture Instance ID")
}

fn record(value: &[u8]) -> StateRecord {
    StateRecord::from_bytes(value.to_vec()).expect("bounded state record")
}
