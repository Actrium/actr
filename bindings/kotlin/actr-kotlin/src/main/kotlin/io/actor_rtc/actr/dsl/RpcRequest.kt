/**
 * Type-safe RPC protocol for compile-time checked request/response pairs.
 *
 * ## Usage
 *
 * ```kotlin
 * // 1. Define your RPC contract
 * object EchoRpc : RpcRequest<EchoRequest, EchoResponse> {
 *     override val routeKey = "echo.EchoService.Echo"
 *     override fun serializeRequest(request: EchoRequest) = request.toByteArray()
 *     override fun deserializeResponse(bytes: ByteArray) = EchoResponse.parseFrom(bytes)
 * }
 *
 * // 2. Call with type safety
 * val response: EchoResponse = ref.call(EchoRpc, EchoRequest.newBuilder().setMessage("hello").build())
 * ```
 *
 * ### With lambdas (inline, no object needed)
 *
 * ```kotlin
 * val response = ref.call(
 *     "echo.EchoService.Echo",
 *     request = EchoRequest.newBuilder().setMessage("hello").build(),
 *     serialize = { it.toByteArray() },
 *     deserialize = { EchoResponse.parseFrom(it) },
 * )
 * ```
 */
package io.actor_rtc.actr.dsl

import io.actor_rtc.actr.PayloadType

/**
 * Type-safe RPC contract that binds a request type to its response type and route.
 *
 * Implement this interface once per RPC method to get compile-time type safety
 * when calling remote actors.
 *
 * @param Req The request message type
 * @param Resp The response message type
 */
interface RpcRequest<Req, Resp> {
    /** RPC route key, e.g., "echo.EchoService.Echo". */
    val routeKey: String

    /** Serialize the request message to bytes. */
    fun serializeRequest(request: Req): ByteArray

    /** Deserialize the response message from bytes. */
    fun deserializeResponse(bytes: ByteArray): Resp
}

/**
 * Perform a type-safe RPC call using an [RpcRequest] contract.
 *
 * Example:
 * ```kotlin
 * val response: EchoResponse = ref.call(EchoRpc, EchoRequest.newBuilder().setMessage("hello").build())
 * ```
 *
 * @param rpc The RPC contract defining route, serialization, and deserialization
 * @param request The request message
 * @param payloadType Transmission type (default: RPC_RELIABLE)
 * @param timeoutMs Timeout in milliseconds (default: 30000)
 * @return The deserialized response
 */
suspend fun <Req, Resp> ActrRef.call(
    rpc: RpcRequest<Req, Resp>,
    request: Req,
    payloadType: PayloadType = PayloadType.RPC_RELIABLE,
    timeoutMs: Long = 30000L,
): Resp {
    val responseBytes = call(rpc.routeKey, payloadType, rpc.serializeRequest(request), timeoutMs)
    return rpc.deserializeResponse(responseBytes)
}

/**
 * Perform an inline type-safe RPC call with lambda-based serialization.
 *
 * This overload is useful when you don't want to define a separate [RpcRequest]
 * object — just pass the serialization lambdas inline.
 *
 * Example:
 * ```kotlin
 * val response = ref.call(
 *     "echo.EchoService.Echo",
 *     request = EchoRequest.newBuilder().setMessage("hello").build(),
 *     serialize = { it.toByteArray() },
 *     deserialize = { EchoResponse.parseFrom(it) },
 * )
 * ```
 *
 * @param routeKey RPC route key
 * @param request The request message
 * @param payloadType Transmission type (default: RPC_RELIABLE)
 * @param timeoutMs Timeout in milliseconds (default: 30000)
 * @param serialize Function to serialize the request to bytes
 * @param deserialize Function to deserialize the response from bytes
 * @return The deserialized response
 */
suspend inline fun <Req, Resp> ActrRef.call(
    routeKey: String,
    request: Req,
    payloadType: PayloadType = PayloadType.RPC_RELIABLE,
    timeoutMs: Long = 30000L,
    crossinline serialize: (Req) -> ByteArray,
    crossinline deserialize: (ByteArray) -> Resp,
): Resp {
    val responseBytes = call(routeKey, payloadType, serialize(request), timeoutMs)
    return deserialize(responseBytes)
}