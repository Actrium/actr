/**
 * Actor-RTC Kotlin SDK
 *
 * This file re-exports all public API for convenient imports.
 *
 * Usage:
 * ```kotlin
 * import io.actor_rtc.actr.dsl.*
 * ```
 *
 * This gives you access to:
 * - Type aliases: ActrSystem, ActrNode, ActrRef, Workload
 * - DSL builders: actrType(), actrId(), dataStream(), workload()
 * - Extensions: String.toActrType(), ActrRef.discover(String), etc.
 * - Utilities: withRetry(), SimpleWorkload, RoutedWorkload
 *
 * For direct access to generated types, use:
 * ```kotlin
 * import io.actor_rtc.actr.*
 * ```
 */
@file:Suppress("unused")

package io.actor_rtc.actr.dsl

// Re-export commonly used types from the generated bindings
// Users can import either:
// - io.actor_rtc.actr.dsl.* for the DSL API
// - io.actor_rtc.actr.* for the raw generated types
