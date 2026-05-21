//! `ServiceHandler` — protoc-gen handler meta-trait.
//!
//! Every `{Service}Handler` trait emitted by `protoc-gen-actrframework`
//! inherits from this trait and binds its `Workload` associated type to the
//! code-generated `{Service}Workload<Self>` wrapper. The `entry!` macro (and
//! anything else downstream of protoc-gen) can therefore recover the
//! concrete `Workload` type from just the handler type — `W = <H as
//! ServiceHandler>::Workload` — without re-deriving it by name mangling.
//!
//! Per Option U γ-unified §4.5 / Phase 6b `protoc-gen` design:
//!
//! ```rust,ignore
//! // Generated
//! pub trait EchoServiceHandler:
//!     actr_framework::ServiceHandler<Workload = EchoServiceWorkload<Self>>
//! { ... }
//! ```
//!
//! The trait is deliberately minimal — the only thing it promises is the
//! mapping from the user's domain-specific handler type to the
//! `Workload`-implementing wrapper that `entry!` will eventually register.
//! No runtime methods are added; `ServiceHandler` is a pure type-level
//! associator.

use crate::Workload;

/// Associator trait emitted by protoc-gen: maps a user-implemented handler
/// type to the generated `{Service}Workload<Self>` wrapper.
///
/// Users never implement this trait by hand; the generator's
/// `{Service}Handler` trait expands into a super-trait bound on it so the
/// framework can recover `type Workload` without name mangling.
///
/// # Design notes
///
/// - `MaybeSendSync + 'static` mirrors the bound on every
///   `{Service}Handler`, so downstream generic code can compose
///   `ServiceHandler` bounds into dispatcher-generic functions without
///   re-stating the auto-trait marker.
/// - The associated type pins a concrete `Workload` impl, not a `dyn
///   Workload`. Static dispatch all the way down — matching the rest of
///   the framework's zero-cost abstractions.
pub trait ServiceHandler: crate::MaybeSendSync + 'static {
    /// Concrete `Workload` wrapper for this handler.
    ///
    /// protoc-gen's `{Service}Handler` trait sets this to the generated
    /// `{Service}Workload<Self>` wrapper, which carries the dispatcher
    /// (via `type Dispatcher = {Service}Dispatcher<Self>`) and inherits
    /// every observation-hook default from `Workload`.
    type Workload: Workload;
}
