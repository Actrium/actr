//! Workload trait - Executable actor workload

use actr_protocol::ActorResult;
use async_trait::async_trait;

use crate::{Context, MessageDispatcher};

/// Workload - Executable Actor workload
///
/// Represents a complete Actor instance, including:
/// - Associated dispatcher type (Dispatcher)
/// - Lifecycle hooks (on_start, on_stop)
///
/// # Design Characteristics
///
/// - **Bidirectional association**: `Workload::Dispatcher` and `MessageDispatcher::Workload` reference each other
/// - **Default implementations**: Lifecycle hooks have default no-op implementations, users can optionally override
/// - **Auto-implementation**: Implemented for wrapper types by code generator
///
/// # Code Generation Example
///
/// ```rust,ignore
/// // User-implemented Handler
/// pub struct MyEchoService { /* ... */ }
///
/// impl EchoServiceHandler for MyEchoService {
///     async fn echo<C: Context>(
///         &self,
///         req: EchoRequest,
///         ctx: &C,
///     ) -> ActorResult<EchoResponse> {
///         // Business logic
///         Ok(EchoResponse { reply: format!("Echo: {}", req.message) })
///     }
/// }
///
/// // Code-generated Workload wrapper
/// pub struct EchoServiceWorkload<T: EchoServiceHandler>(pub T);
///
/// impl<T: EchoServiceHandler> Workload for EchoServiceWorkload<T> {
///     type Dispatcher = EchoServiceRouter<T>;
/// }
/// ```
#[async_trait]
pub trait Workload: Send + Sync + 'static {
    /// Associated dispatcher type
    type Dispatcher: MessageDispatcher<Workload = Self>;

    /// Lifecycle hook: Called when Actor starts
    ///
    /// # Default Implementation
    ///
    /// Default is a no-op, users can optionally override to perform initialization logic.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// async fn on_start<C: Context>(&self, ctx: &C) -> ActorResult<()> {
    ///     tracing::info!("Actor {} started", ctx.self_id());
    ///     // Initialize resources
    ///     Ok(())
    /// }
    /// ```
    async fn on_start<C: Context>(&self, _ctx: &C) -> ActorResult<()> {
        Ok(())
    }

    /// Lifecycle hook: Called when Actor stops
    ///
    /// # Default Implementation
    ///
    /// Default is a no-op, users can optionally override to perform cleanup logic.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// async fn on_stop<C: Context>(&self, ctx: &C) -> ActorResult<()> {
    ///     tracing::info!("Actor {} stopping", ctx.self_id());
    ///     // Cleanup resources
    ///     Ok(())
    /// }
    /// ```
    async fn on_stop<C: Context>(&self, _ctx: &C) -> ActorResult<()> {
        Ok(())
    }
}
