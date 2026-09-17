//! Exercise caller-owned output failures without deciding format expectations.
//!
//! Tests choose short writes, a failure boundary, cancellation or a failed flush.
//! Retained bytes and flush counts let each format check its own completion rule.
//!
//! The sink can also panic after a chosen prefix to exercise unwinding. Bytes already
//! accepted remain available for the test to inspect, while the injected error kind
//! and cancellation token determine how later writes or flushes fail. Production
//! encoders still own their byte limits, completion records and error conversion.

use crate::CancellationToken;
use std::io::{self, Write};

pub(crate) struct Sink<'a> {
    pub(crate) bytes: Vec<u8>,
    pub(crate) short: usize,
    pub(crate) fail_after: usize,
    pub(crate) panic_after: usize,
    pub(crate) failure: io::ErrorKind,
    pub(crate) fail_flush: bool,
    pub(crate) flushes: usize,
    pub(crate) cancel: Option<&'a CancellationToken>,
}

impl Default for Sink<'_> {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            short: usize::MAX,
            fail_after: usize::MAX,
            panic_after: usize::MAX,
            failure: io::ErrorKind::BrokenPipe,
            fail_flush: false,
            flushes: 0,
            cancel: None,
        }
    }
}

impl Write for Sink<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        assert!(
            self.bytes.len() < self.panic_after,
            "controlled writer panic"
        );
        if self.bytes.len() >= self.fail_after {
            return Err(self.failure.into());
        }
        let length = bytes
            .len()
            .min(self.short)
            .min(self.fail_after - self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..length]);
        if let Some(cancel) = self.cancel {
            cancel.cancel();
        }
        Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.fail_flush {
            Err(io::ErrorKind::Other.into())
        } else {
            Ok(())
        }
    }
}
