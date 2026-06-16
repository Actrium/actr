/**
 * Actrium Kotlin SDK
 *
 * A Kotlin-idiomatic wrapper for the Actrium framework.
 *
 * Example usage:
 * ```kotlin
 * // Create and start a package-backed actor
 * val node = ActrNode.fromPackageFile("config.toml", "dist/app.actr")
 * val ref = node.start()
 *
 * // Discover and call remote services
 * val echoService = ref.discoverOne("acme:EchoService:1.0.0")
 * val response = ref.call("echo.EchoService.Echo", request)
 *
 * // Send data stream
 * ref.sendStream(target) {
 *     streamId = "stream-001"
 *     sequence = 0uL
 *     payload = data
 *     metadata {
 *         "content-type" to "application/octet-stream"
 *     }
 * }
 *
 * // Clean shutdown
 * ref.stop()
 * ```
 */
package io.actrium.actr.dsl

import io.actrium.actr.ActrException
import io.actrium.actr.ActrId
import io.actrium.actr.ActrRefWrapper
import io.actrium.actr.ActrType
import io.actrium.actr.DynamicWorkload
import io.actrium.actr.NetworkEventHandleWrapper
import io.actrium.actr.PayloadType
import io.actrium.actr.WorkloadLifecycleBridge
import io.actrium.actr.ActrNode as ActrNodeGenerated
import java.net.URL

// ============================================================================
// Type Aliases — provide DSL-friendly names for remaining generated types
// ============================================================================

/** Handle for network event callbacks. Used for platform integration. */
typealias NetworkEventHandle = NetworkEventHandleWrapper

/** Workload callback interface for handling lifecycle events. */
typealias Workload = WorkloadLifecycleBridge

// ============================================================================
// ActrNode — high-level wrapper with workload retention
// ============================================================================

/**
 * Entry point for creating and starting ACTR nodes.
 *
 * This is a high-level wrapper around the UniFFI-generated [ActrNodeGenerated]
 * that manages workload lifecycle and retains references to prevent premature
 * garbage collection.
 *
 * Use [ActrNode.fromPackageFile] or [ActrNode.linked] to create an instance.
 */
class ActrNode private constructor(
    private val inner: ActrNodeGenerated,
    private val retainedWorkload: DynamicWorkload? = null,
) : AutoCloseable {
    /** Close the underlying node, releasing native resources. */
    override fun close() = inner.close()

    companion object {
        /**
         * Create a package-backed node from config and package file paths.
         *
         * Example:
         * ```kotlin
         * val node = ActrNode.fromPackageFile("config.toml", "dist/app.actr")
         * val ref = node.start()
         * ```
         *
         * @param configPath Path to the TOML configuration file
         * @param packagePath Path to the `.actr` package file
         * @return A new ActrNode instance
         * @throws ActrException.Config if the config file is invalid
         */
        suspend fun fromPackageFile(
            configPath: String,
            packagePath: String,
        ): ActrNode {
            val inner = ActrNodeGenerated.newFromPackageFile(configPath, packagePath)
            return ActrNode(inner, null)
        }

        /**
         * Create a linked/static node from config, explicit actor identity, and a
         * Kotlin-provided workload.
         *
         * Use this when workload logic lives in Kotlin instead of a packaged `.actr`
         * guest. The returned [ActrNode] retains the [workload] reference to prevent
         * premature garbage collection.
         *
         * Example:
         * ```kotlin
         * val workload = dynamicWorkload(myLifecycle)
         * val node = ActrNode.linked("config.toml", myType, workload)
         * val ref = node.start()
         * ```
         *
         * @param configPath Path to the TOML configuration file
         * @param actorType The actor's type identity
         * @param workload The composed workload (lifecycle + optional observers)
         * @return A new ActrNode instance that retains the workload
         * @throws ActrException.Config if the config file is invalid
         */
        suspend fun linked(
            configPath: String,
            actorType: ActrType,
            workload: DynamicWorkload,
        ): ActrNode {
            val inner = ActrNodeGenerated.newFromLinkedWorkload(configPath, actorType, workload)
            return ActrNode(inner, workload)
        }

        /**
         * Create a package-backed node from config and package file URLs.
         *
         * Validates that both URLs are file URLs before delegating to
         * [fromPackageFile] with the URL paths.
         *
         * @param configURL File URL to the TOML configuration file
         * @param packageURL File URL to the `.actr` package file
         * @return A new ActrNode instance
         * @throws IllegalArgumentException if either URL is not a file URL
         */
        suspend fun fromPackageFile(
            configURL: URL,
            packageURL: URL,
        ): ActrNode {
            require(configURL.protocol == "file") {
                "configURL must be a file URL, got: $configURL"
            }
            require(packageURL.protocol == "file") {
                "packageURL must be a file URL, got: $packageURL"
            }
            return fromPackageFile(configURL.path, packageURL.path)
        }

        /**
         * Create a linked node from a config file URL.
         *
         * @param configURL File URL to the TOML configuration file
         * @param actorType The actor's type identity
         * @param workload The composed workload
         * @return A new ActrNode instance that retains the workload
         * @throws IllegalArgumentException if the URL is not a file URL
         */
        suspend fun linked(
            configURL: URL,
            actorType: ActrType,
            workload: DynamicWorkload,
        ): ActrNode {
            require(configURL.protocol == "file") {
                "config URL must be a file URL, got: $configURL"
            }
            return linked(configURL.path, actorType, workload)
        }
    }

    /**
     * Create a network event handle for platform callbacks.
     *
     * This handle is used to notify the actor runtime about network state changes,
     * app lifecycle transitions, and explicit cleanup/reconnect operations, which
     * are important for WebRTC connection management on mobile platforms.
     *
     * Example:
     * ```kotlin
     * val networkHandle = node.createNetworkEventHandle()
     *
     * // Notify full network path change
     * networkHandle.handleNetworkPathChanged(
     *     NetworkSnapshot(
     *         sequence = 1uL,
     *         availability = NetworkAvailability.AVAILABLE,
     *         transport = NetworkTransportFlags(wifi = true, cellular = false, ethernet = false, vpn = false, other = false),
     *         isExpensive = false,
     *         isConstrained = false,
     *     )
     * )
     * ```
     *
     * @return A new NetworkEventHandle instance
     * @throws ActrException if the handle cannot be created
     */
    suspend fun createNetworkEventHandle(): NetworkEventHandle = inner.createNetworkEventHandle()

    /**
     * Start the actor and return a running reference.
     *
     * The returned [ActrRef] retains the workload (if any) to prevent premature
     * garbage collection.
     *
     * @return A running [ActrRef] instance
     * @throws ActrException if startup fails
     */
    suspend fun start(): ActrRef {
        val ref = inner.start()
        return ActrRef(ref, retainedWorkload)
    }

    /**
     * Execute a block with a started actor, ensuring proper cleanup.
     *
     * The actor is automatically shut down after the block completes, even if
     * an exception is thrown.
     *
     * Example:
     * ```kotlin
     * node.withStartedActor { ref ->
     *     val target = ref.discoverOne("acme:EchoService:1.0.0")
     *     val response = ref.call("echo.EchoService.Echo", payload)
     * }
     * // Actor is automatically shut down after the block
     * ```
     */
    suspend fun <T> withStartedActor(block: suspend (ActrRef) -> T): T {
        val ref = start()
        return try {
            block(ref)
        } finally {
            try {
                ref.shutdown()
                ref.waitForShutdown()
            } catch (_: Exception) {
                // Ignore cleanup errors
            }
        }
    }
}

// ============================================================================
// ActrRef — high-level wrapper with workload retention
// ============================================================================

/**
 * Reference to a running actor.
 *
 * This is a high-level wrapper around the UniFFI-generated [ActrRefWrapper]
 * that provides:
 * - Convenience methods with default parameters
 * - Workload retention to prevent premature garbage collection
 * - Scoped lifecycle helpers
 *
 * Methods:
 * - [call] / [tell] — RPC communication
 * - [discover] / [discoverOne] — Service discovery
 * - [stop] / [shutdown] — Graceful shutdown
 */
class ActrRef internal constructor(
    private val inner: ActrRefWrapper,
    internal val retainedWorkload: DynamicWorkload? = null,
) : AutoCloseable {
    /** Close the underlying reference, releasing native resources. */
    override fun close() = inner.close()

    /** Get the actor's unique identifier. */
    fun actorId(): ActrId = inner.actorId()

    /**
     * Perform an RPC call with explicit parameters.
     *
     * For most use cases, prefer the convenience overload:
     * ```kotlin
     * ref.call("echo.EchoService.Echo", requestPayload)
     * ```
     */
    suspend fun call(
        routeKey: String,
        payloadType: PayloadType,
        requestPayload: ByteArray,
        timeoutMs: Long,
    ): ByteArray = inner.call(routeKey, payloadType, requestPayload, timeoutMs)

    /**
     * Send a one-way message (fire-and-forget) with explicit parameters.
     *
     * For most use cases, prefer the convenience overload:
     * ```kotlin
     * ref.tell("echo.EchoService.Notify", messagePayload)
     * ```
     */
    suspend fun tell(
        routeKey: String,
        payloadType: PayloadType,
        messagePayload: ByteArray,
    ) = inner.tell(routeKey, payloadType, messagePayload)

    /** Discover actors of the specified type. */
    suspend fun discover(
        targetType: ActrType,
        count: UInt,
    ): List<ActrId> = inner.discover(targetType, count)

    /** Check if the actor is shutting down. */
    fun isShuttingDown(): Boolean = inner.isShuttingDown()

    /** Whether this actor reference is still valid (not destroyed). */
    val isActive: Boolean
        get() = !isShuttingDown()

    /** Trigger shutdown. */
    fun shutdown() = inner.shutdown()

    /** Wait for shutdown to complete. */
    suspend fun waitForShutdown() = inner.waitForShutdown()

    /**
     * Shut down the actor and wait for it to terminate.
     *
     * This is the recommended way to stop an actor. Equivalent to:
     * ```kotlin
     * ref.shutdown()
     * ref.waitForShutdown()
     * ```
     */
    suspend fun stop() {
        shutdown()
        waitForShutdown()
    }
}

// ============================================================================
// Top-Level Convenience Functions
// ============================================================================

/**
 * Create an ActrNode from a config file and package file (top-level function).
 *
 * Example:
 * ```kotlin
 * val node = createActrNode("config.toml", "dist/app.actr")
 * ```
 *
 * @param configPath Path to the TOML configuration file
 * @param packagePath Path to the `.actr` package file
 * @return A new ActrNode instance
 * @throws ActrException.Config if the config file is invalid
 */
suspend fun createActrNode(
    configPath: String,
    packagePath: String,
): ActrNode = ActrNode.fromPackageFile(configPath, packagePath)

/**
 * Create an ActrNode backed by a linked dynamic workload (top-level function).
 *
 * @param configPath Path to the TOML configuration file
 * @param actorType The actor's type identity
 * @param workload The composed workload
 * @return A new ActrNode instance that retains the workload
 */
suspend fun linked(
    configPath: String,
    actorType: ActrType,
    workload: DynamicWorkload,
): ActrNode = ActrNode.linked(configPath, actorType, workload)

// ============================================================================
// ActrRef Extensions
// ============================================================================

/**
 * Discover actors of the specified type using a type string.
 *
 * @param typeString Actor type in "manufacturer:name:version" format (e.g., "acme:EchoService:1.0.0")
 * @param count Maximum number of candidates to return (default: 1)
 * @return List of discovered actor IDs
 */
suspend fun ActrRef.discover(
    typeString: String,
    count: UInt = 1u,
): List<ActrId> = discover(typeString.toActrType(), count)

/**
 * Discover a single actor of the specified type.
 *
 * @param typeString Actor type in "manufacturer:name:version" format
 * @return The first discovered actor ID, or null if none found
 */
suspend fun ActrRef.discoverOne(typeString: String): ActrId? = discover(typeString, 1u).firstOrNull()

/**
 * Discover a single actor of the specified type.
 *
 * @param type Actor type
 * @return The first discovered actor ID, or null if none found
 */
suspend fun ActrRef.discoverOne(type: ActrType): ActrId? = discover(type, 1u).firstOrNull()

/** Await shutdown completion. Alias for [ActrRef.waitForShutdown]. */
suspend fun ActrRef.awaitShutdown() {
    waitForShutdown()
}

// ============================================================================
// SimpleWorkload Extensions
// ============================================================================

/**
 * Send a DataStream built with DSL syntax.
 *
 * Example:
 * ```kotlin
 * workload.sendStream(targetId) {
 *     streamId = "my-stream"
 *     sequence = 0uL
 *     payload = "Hello".toByteArray()
 *     metadata {
 *         "key1" to "value1"
 *         "key2" to "value2"
 *     }
 * }
 * ```
 */
suspend fun SimpleWorkload.sendStream(
    target: ActrId,
    builder: DataStreamBuilder.() -> Unit,
) {
    val dataStream = DataStreamBuilder().apply(builder).build()
    sendDataStream(target, dataStream)
}