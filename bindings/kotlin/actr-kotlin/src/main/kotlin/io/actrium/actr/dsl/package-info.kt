/**
 * Actrium Kotlin SDK
 *
 * This file re-exports all public API for convenient imports.
 *
 * Usage:
 * ```kotlin
 * import io.actrium.actr.dsl.*
 * ```
 *
 * This gives you access to:
 * - Type aliases: ActrNode, ActrRef, Workload, ActrContext, RpcEnvelope, LogCallback,
 *   DataStreamCallback, MediaSample, MediaTrackCallback, MediaType, OpusEncoder
 * - DSL builders: actrType(), actrId(), dataChunk(), workload()
 * - Extensions: String.toActrType(), ActrRef.discover(String), etc.
 * - Utilities: withRetry(), withStartedActor(), SimpleWorkload, RoutedWorkload
 * - Logging: setLogCallback()
 * - Manifest: Manifest class, resolveManifestDependency(),
 *   resolveManifestDependencyAliasList(), resolveManifestPackageActrType()
 *
 * For direct access to generated types, use:
 * ```kotlin
 * import io.actrium.actr.*
 * ```
 */
@file:Suppress("unused")

package io.actrium.actr.dsl

// Re-export commonly used types from the generated bindings
// Users can import either:
// - io.actrium.actr.dsl.* for the DSL API
// - io.actrium.actr.* for the raw generated types
