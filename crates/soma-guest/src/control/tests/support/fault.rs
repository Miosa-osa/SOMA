use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

use crate::{ControlIo, HostControlIo};

#[derive(Clone, Copy)]
pub(crate) enum Direction {
    Read,
    Write,
}

#[derive(Clone, Default)]
pub(crate) struct Traffic {
    read: Arc<AtomicUsize>,
    written: Arc<AtomicUsize>,
}

pub(crate) struct FaultIo<I> {
    inner: I,
    direction: Direction,
    fail_at: Option<usize>,
    progress: usize,
    traffic: Traffic,
}

impl<I> FaultIo<I> {
    pub(crate) fn new(inner: I, direction: Direction, fail_at: Option<usize>) -> (Self, Traffic) {
        let traffic = Traffic::default();
        (
            Self {
                inner,
                direction,
                fail_at,
                progress: 0,
                traffic: traffic.clone(),
            },
            traffic,
        )
    }

    fn should_fail(&self) -> bool {
        self.fail_at == Some(self.progress)
    }
}

impl Traffic {
    pub(crate) fn read(&self) -> usize {
        self.read.load(Ordering::SeqCst)
    }

    pub(crate) fn written(&self) -> usize {
        self.written.load(Ordering::SeqCst)
    }
}

impl<I: ControlIo> ControlIo for FaultIo<I> {
    type Error = ();

    fn read_exact(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<(), Self::Error> {
        if !matches!(self.direction, Direction::Read) {
            self.inner.read_exact(bytes, deadline).map_err(|_| ())?;
            self.traffic.read.fetch_add(bytes.len(), Ordering::SeqCst);
            return Ok(());
        }
        for byte in bytes {
            if self.should_fail() {
                return Err(());
            }
            self.inner
                .read_exact(core::slice::from_mut(byte), deadline)
                .map_err(|_| ())?;
            self.progress += 1;
            self.traffic.read.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), Self::Error> {
        if !matches!(self.direction, Direction::Write) {
            self.inner.write_all(bytes, deadline).map_err(|_| ())?;
            self.traffic
                .written
                .fetch_add(bytes.len(), Ordering::SeqCst);
            return Ok(());
        }
        for byte in bytes {
            if self.should_fail() {
                return Err(());
            }
            self.inner
                .write_all(core::slice::from_ref(byte), deadline)
                .map_err(|_| ())?;
            self.progress += 1;
            self.traffic.written.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    fn poison(&mut self) {
        self.inner.poison();
    }
}

impl<I: HostControlIo> HostControlIo for FaultIo<I> {
    fn commit_repair(&mut self, deadline: Instant) -> Result<(), Self::Error> {
        self.inner.commit_repair(deadline).map_err(|_| ())
    }
}
