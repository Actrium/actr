//! In-memory dispatch scheduling layer (conflict-key routing + concurrency
//! budget + back-pressure).
//!
//! ## Where this sits — and how it differs from the SQLite mailbox
//!
//! The node has two very different "queues" and they must not be conflated:
//!
//! * The **persistent mailbox** ([`actr_runtime_mailbox`]) is the *durable*,
//!   at-least-once buffer that lives on the wire boundary. It survives process
//!   restarts, drives the reply-before-ack crash window, and provides transport
//!   back-pressure by holding messages on disk. It is upstream of everything
//!   here.
//!
//! * This **dispatch scheduler** is a purely *in-memory* layer that sits between
//!   the node entry loops (which dequeue from the durable mailbox) and the
//!   per-actor serial command runner ([`crate::executor`]). It never persists
//!   anything. Its only jobs are:
//!     1. project each inbound RPC to a [`conflict_key::ConflictKey`],
//!     2. keep same-key messages strictly FIFO / one-in-flight,
//!     3. let distinct-key messages run concurrently up to a budget `C`,
//!     4. apply a bounded queue `M` whose full state produces back-pressure that
//!        propagates up to the entry loop → the durable mailbox / SCTP flow
//!        control (design doc §8.3).
//!
//! The name deliberately avoids "mailbox" so the durable and in-memory layers
//! stay legible as distinct concepts.
//!
//! ## Default-off, serial-safe
//!
//! The layer is gated (`HyperConfig::dispatch_concurrency`, default `None` =
//! off) and additionally treats every *undeclared* method as a global
//! [`conflict_key::ConflictKey::Serial`] barrier. Both together mean the default
//! behaviour is bit-for-bit the B1 serial runner: at most one dispatch in
//! flight, in arrival order. Concurrency only appears after a consumer both
//! turns the gate on *and* declares conflict keys for specific methods.
//!
//! ## Scope (B2)
//!
//! The scheduler only routes RPC **Dispatch** work. The Direction=Response
//! bypass (`gate.rs` pending_requests) and the DataStream path
//! (`data_stream_registry`) are untouched — DataStream's per-stream serialization
//! is the `conflict_key = stream_id` special case, left unified for a later
//! milestone.

pub(crate) mod conflict_key;
pub(crate) mod scheduler;

pub use conflict_key::{ConflictKeyError, ConflictKeySpec, ConflictKeySpecBuilder, KeySource};

#[cfg(test)]
#[path = "conflict_key_tests.rs"]
mod conflict_key_tests;

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod scheduler_tests;
