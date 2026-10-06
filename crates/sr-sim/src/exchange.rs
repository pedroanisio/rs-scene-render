//! What the solvers of a coupled group pass each other.
//!
//! A group of solvers that depend on one another (rigid bodies and the water they move
//! through) cannot each be replayed alone by asking the others again: that would restore
//! their checkpoints in turn. Instead each writes the small records the others need into
//! an append-only log, once, and every replay reads them from it. A record is never
//! changed: writing a step again must reproduce it to the last bit, because a replay that
//! does not is a solver that is no longer deterministic, and the other members have already
//! used the first record.
//!
//! Records are small by design (a few numbers per body per step); grid fields do not belong
//! here. The log has a byte budget, and exceeding it is an error: nothing is dropped, since
//! a record dropped could not be replayed.

use std::collections::BTreeMap;
use std::fmt::Debug;

/// A record that can be compared to its own replay.
pub trait Record: Copy + Debug {
    /// Whether `other` is the same record to the last bit (floating-point numbers by their
    /// bits, so that `0.0` and `-0.0` differ and `NaN` equals itself).
    fn same(&self, other: &Self) -> bool;
}

/// What `put` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Put {
    /// New records were stored.
    Stored,
    /// The step was already there and the records reproduced it exactly.
    Replayed,
}

/// Bytes charged for each stored step, on top of its records.
const ENTRY_BYTES: usize = 64;

/// An append-only log of records by channel and step.
#[derive(Debug)]
pub struct ExchangeLog<R: Record> {
    budget: usize,
    bytes: usize,
    steps: BTreeMap<(u32, u64), Vec<R>>,
}

impl<R: Record> ExchangeLog<R> {
    /// An empty log that may use `budget` bytes.
    pub fn new(budget: usize) -> Self {
        ExchangeLog { budget, bytes: 0, steps: BTreeMap::new() }
    }

    fn charge(records: usize) -> usize {
        ENTRY_BYTES + records * std::mem::size_of::<R>()
    }

    /// Store the `records` of `channel` at `step`, or check that they are the ones already
    /// stored. A different replay, or going over the budget, is an error that changes nothing.
    pub fn put(&mut self, channel: u32, step: u64, records: &[R]) -> Result<Put, String> {
        if let Some(have) = self.steps.get(&(channel, step)) {
            let same = have.len() == records.len() && have.iter().zip(records).all(|(a, b)| a.same(b));
            return if same {
                Ok(Put::Replayed)
            } else {
                Err(format!(
                    "exchange channel {channel} step {step} diverges from its first record: {} then {}",
                    summary(have),
                    summary(records)
                ))
            };
        }
        let total = self.bytes + Self::charge(records.len());
        if total > self.budget {
            return Err(format!("exchange log of {total} bytes exceeds its budget of {}", self.budget));
        }
        self.bytes = total;
        self.steps.insert((channel, step), records.to_vec());
        Ok(Put::Stored)
    }

    /// The records of `channel` at `step`, if they have been written.
    pub fn get(&self, channel: u32, step: u64) -> Option<&[R]> {
        self.steps.get(&(channel, step)).map(Vec::as_slice)
    }

    /// Steps stored, over all channels.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether nothing has been stored.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Bytes charged for what is stored.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The last step stored for `channel`.
    pub fn last_step(&self, channel: u32) -> Option<u64> {
        self.steps.range((channel, 0)..=(channel, u64::MAX)).next_back().map(|((_, step), _)| *step)
    }
}

fn summary<R: Debug>(records: &[R]) -> String {
    match records {
        [] => "no records".into(),
        [one] => format!("{one:?}"),
        [first, ..] => format!("{} records from {first:?}", records.len()),
    }
}
