//! Bounded buffered transfers over a borrowed scratch-file effect authority.
use super::MAX_FRAME_BYTES;
use crate::effects::Effects;
use crate::resources::allocate;
use crate::{CancellationToken, Error};

// Small records share bounded I/O buffers. Large records copy through the same
// buffers; work decreases by copied bytes and never depends on file cardinality.

// One borrowed effect authority per call; neither buffers nor cursors retain it.
pub(in crate::execution) struct Io<'call, 'db> {
    scratch: &'call mut crate::scratch::Scratch<'db>,
    pub(in crate::execution) cancel: &'call CancellationToken,
    effects: &'call mut Effects,
}

impl<'call, 'db> Io<'call, 'db> {
    pub(super) fn reset(&mut self, slot: usize) -> Result<(), Error> {
        self.scratch.reset(slot, self.cancel, self.effects)
    }

    pub(in crate::execution) fn new(
        scratch: &'call mut crate::scratch::Scratch<'db>,
        cancel: &'call CancellationToken,
        effects: &'call mut Effects,
    ) -> Self {
        Self {
            scratch,
            cancel,
            effects,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::execution) struct ReadAt {
    pub(in crate::execution) slot: usize,
    pub(in crate::execution) offset: u64,
    pub(in crate::execution) limit: u64,
}

pub(in crate::execution) const IO_BYTES: usize = 65_536;

pub(in crate::execution) struct ReadBuffer {
    bytes: Vec<u8>,
    start: u64,
    used: usize,
    slot: Option<usize>,
}

impl ReadBuffer {
    #[cfg(test)]
    pub(super) fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
    }

    #[cfg(test)]
    pub(super) fn buffered_bytes(&self) -> usize {
        self.used
    }

    pub(super) fn new(charge: u64) -> Result<Self, Error> {
        let mut bytes = allocate(IO_BYTES, IO_BYTES, "group read buffer", charge)?;
        bytes.resize(IO_BYTES, 0);
        Ok(Self {
            bytes,
            start: 0,
            used: 0,
            slot: None,
        })
    }

    pub(in crate::execution) fn clear(&mut self) {
        self.used = 0;
    }

    pub(in crate::execution) fn read(
        &mut self,
        at: ReadAt,
        mut output: &mut [u8],
        io: &mut Io<'_, '_>,
    ) -> Result<(), Error> {
        let ReadAt {
            slot,
            mut offset,
            limit,
        } = at;
        if slot >= 2 {
            return Err(Error::Corrupt("group input file slot"));
        }

        if self.slot != Some(slot) {
            self.used = 0;
            self.slot = Some(slot);
        }
        let end = offset
            .checked_add(output.len() as u64)
            .ok_or(Error::Corrupt("group buffered read extent"))?;
        if end > limit || output.len() > MAX_FRAME_BYTES {
            return Err(Error::Corrupt("group read exceeds run boundary"));
        }
        while !output.is_empty() {
            io.cancel.check()?;
            let buffered_end = self
                .start
                .checked_add(self.used as u64)
                .ok_or(Error::Corrupt("group read buffer end"))?;
            if offset < self.start || offset >= buffered_end {
                let used =
                    usize::try_from((limit - offset).min(IO_BYTES as u64)).expect("bounded read");
                self.used = 0;
                io.scratch
                    .read(slot, &mut self.bytes[..used], offset, io.cancel, io.effects)?;
                self.start = offset;
                self.used = used;
            }
            let at = usize::try_from(offset - self.start).expect("offset within read buffer");
            let take = output.len().min(self.used - at);
            assert!(take != 0, "a nonempty bounded read advances");
            output[..take].copy_from_slice(&self.bytes[at..at + take]);
            output = &mut output[take..];
            offset += take as u64;
        }
        Ok(())
    }
}

pub(in crate::execution) struct WriteBuffer {
    bytes: Vec<u8>,
    offset: u64,
}

impl WriteBuffer {
    #[cfg(test)]
    pub(super) fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(super) fn new(charge: u64) -> Result<Self, Error> {
        Ok(Self {
            bytes: allocate(IO_BYTES, IO_BYTES, "group write buffer", charge)?,
            offset: 0,
        })
    }

    pub(in crate::execution) fn position(&self) -> Result<u64, Error> {
        self.offset
            .checked_add(self.bytes.len() as u64)
            .ok_or(Error::Corrupt("group write position"))
    }

    pub(in crate::execution) fn append(
        &mut self,
        slot: usize,
        mut input: &[u8],
        io: &mut Io<'_, '_>,
    ) -> Result<(), Error> {
        if slot >= 2 {
            return Err(Error::Corrupt("group output file slot"));
        }

        if input.len() > MAX_FRAME_BYTES {
            return Err(Error::Corrupt("group write work bound"));
        }
        while !input.is_empty() {
            io.cancel.check()?;
            if self.bytes.len() == IO_BYTES {
                self.flush(slot, io)?;
            }
            let take = input.len().min(IO_BYTES - self.bytes.len());
            self.bytes.extend_from_slice(&input[..take]);
            input = &input[take..];
        }
        Ok(())
    }

    pub(in crate::execution) fn flush(
        &mut self,
        slot: usize,
        io: &mut Io<'_, '_>,
    ) -> Result<(), Error> {
        if self.bytes.is_empty() {
            return Ok(());
        }
        let next = self.position()?;
        io.scratch
            .write(slot, self.offset, &self.bytes, io.cancel, io.effects)?;
        self.offset = next;
        self.bytes.clear();
        Ok(())
    }

    pub(in crate::execution) fn begin_file(&mut self) {
        assert!(self.bytes.is_empty(), "flush before switching output files");
        self.offset = 0;
    }
}
