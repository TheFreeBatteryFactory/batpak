use crate::guard::{Denial, GateSet, Receipt};
use serde::{Deserialize, Serialize};

/// Bypass types for skipping gate evaluation with an auditable reason.
pub mod bypass;
pub use bypass::{BypassReason, BypassReceipt};

/// `Proposal<T>`: wraps a value for gate evaluation.
pub struct Proposal<T>(
    /// The payload to be evaluated and committed.
    pub(crate) T,
);

/// `Committed<T>`: proof that an event was persisted.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Committed<T> {
    /// The committed event payload.
    pub payload: T,
    /// Unique identifier assigned to this event by the store.
    #[serde(with = "crate::wire::u128_bytes")]
    pub event_id: u128,
    /// Monotonically increasing sequence number within the stream.
    pub sequence: u64,
    /// Content hash of the committed payload (blake3, or all-zeros if feature is off).
    pub hash: [u8; 32], // blake3, or [0u8;32] if feature off
}

/// `Pipeline<Ctx>`: evaluate gates then commit.
pub struct Pipeline<Ctx> {
    /// The set of gates applied during proposal evaluation.
    gates: GateSet<Ctx>,
}

impl<T> Proposal<T> {
    /// Wraps `payload` in a new `Proposal` ready for gate evaluation.
    pub fn new(payload: T) -> Self {
        Self(payload)
    }

    /// Returns a reference to the wrapped payload without consuming the proposal.
    pub fn payload(&self) -> &T {
        &self.0
    }

    /// Transforms the wrapped payload, producing a `Proposal` of a different type.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Proposal<U> {
        Proposal(f(self.0))
    }
}

impl<Ctx> Pipeline<Ctx> {
    /// Creates a new `Pipeline` backed by the given gate set.
    pub fn new(gates: GateSet<Ctx>) -> Self {
        Self { gates }
    }

    /// Runs all gates against `ctx`; returns a `Receipt` on success or the first `Denial`.
    ///
    /// # Errors
    /// Returns the first `Denial` produced by any gate in the pipeline's gate set.
    pub fn evaluate<T>(&self, ctx: &Ctx, proposal: Proposal<T>) -> Result<Receipt<T>, Denial> {
        self.gates.evaluate(ctx, proposal)
    }

    /// commit: generic over error type E. Pipeline doesn't know about StoreError.
    /// Products pass a closure that calls store.append() and wraps the result.
    ///
    /// # Errors
    /// Returns `Err(E)` if the caller-supplied `commit_fn` closure fails.
    pub fn commit<T, E>(
        &self,
        receipt: Receipt<T>,
        commit_fn: impl FnOnce(T) -> Result<Committed<T>, E>,
    ) -> Result<Committed<T>, E> {
        let (payload, _gate_names) = receipt.into_parts();
        commit_fn(payload)
    }

    /// bypass: skip gates with an auditable reason.
    /// [FILE:src/pipeline/bypass.rs]
    pub fn bypass<T>(proposal: Proposal<T>, reason: &'static dyn BypassReason) -> BypassReceipt<T> {
        BypassReceipt {
            payload: proposal.0,
            reason: reason.name(),
            justification: reason.justification(),
        }
    }

    /// commit_bypass: persist a bypassed proposal through the same commit path.
    /// Mirrors commit() but takes a BypassReceipt instead of a Receipt.
    ///
    /// # Errors
    /// Returns `Err(E)` if the caller-supplied `commit_fn` closure fails.
    pub fn commit_bypass<T, E>(
        receipt: BypassReceipt<T>,
        commit_fn: impl FnOnce(T) -> Result<Committed<T>, E>,
    ) -> Result<Committed<T>, E> {
        commit_fn(receipt.into_payload())
    }
}
