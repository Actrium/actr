//! Shell Workload — empty placeholder for dynclib executor mode
//!
//! When a dynclib executor adapter is attached via `ActrNode::with_executor()`,
//! incoming messages are dispatched through the native shared library instead of the
//! native `W::Dispatcher::dispatch` path. This shell workload satisfies the type
//! parameter requirement of `ActrNode<W: Workload>` while never actually handling
//! messages.

use actr_framework::{Context, MessageDispatcher, Workload};
use actr_protocol::{ActorResult, ActrError, RpcEnvelope};
use async_trait::async_trait;
use bytes::Bytes;

/// No-op workload used as placeholder when dynclib executor handles all dispatch
#[derive(Default)]
pub struct ShellWorkload;

/// No-op dispatcher — returns error if somehow reached
pub struct ShellDispatcher;

#[async_trait]
impl MessageDispatcher for ShellDispatcher {
    type Workload = ShellWorkload;

    async fn dispatch<C: Context>(
        _workload: &Self::Workload,
        envelope: RpcEnvelope,
        _ctx: &C,
    ) -> ActorResult<Bytes> {
        // This should never be reached when dynclib executor is configured
        Err(ActrError::UnknownRoute(format!(
            "ShellWorkload: dispatch should not be called in dynclib mode (route_key={})",
            envelope.route_key
        )))
    }
}

impl Workload for ShellWorkload {
    type Dispatcher = ShellDispatcher;
}
