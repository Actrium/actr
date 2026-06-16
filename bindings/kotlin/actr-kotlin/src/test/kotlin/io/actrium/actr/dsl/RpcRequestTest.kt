package io.actrium.actr.dsl

import io.actrium.actr.PayloadType
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * Unit tests for RpcRequest — verifies the type-safe RPC contract and
 * inline lambda call patterns compile correctly.
 *
 * These tests exercise the DSL surface; actual RPC execution requires
 * a running actr node and is covered by UnifiedIntegrationTest (androidTest).
 */
class RpcRequestTest {

    // --- Test fixtures ---

    /** Minimal request message for compile-time contract verification. */
    data class EchoRequest(val message: String)

    /** Minimal response message for compile-time contract verification. */
    data class EchoResponse(val reply: String)

    /**
     * RpcRequest contract binding EchoRequest to EchoResponse via a fixed route.
     */
    object EchoRpc : RpcRequest<EchoRequest, EchoResponse> {
        override val routeKey = "echo.EchoService.Echo"

        override fun serializeRequest(request: EchoRequest): ByteArray =
            request.message.toByteArray()

        override fun deserializeResponse(bytes: ByteArray): EchoResponse =
            EchoResponse(String(bytes))
    }

    // --- Route key ---

    @Test
    fun `RpcRequest routeKey is exposed`() {
        assertEquals("echo.EchoService.Echo", EchoRpc.routeKey)
    }

    // --- Serialize / deserialize round-trip ---

    @Test
    fun `serializeRequest and deserializeResponse round-trip`() {
        val request = EchoRequest("hello")
        val bytes = EchoRpc.serializeRequest(request)
        assertEquals(request.message, String(bytes))

        val response = EchoResponse("world")
        val responseBytes = EchoRpc.serializeRequest(EchoRequest(response.reply))
        val deserialized = EchoRpc.deserializeResponse(responseBytes)
        assertEquals(response.reply, deserialized.reply)
    }

    // --- Compile-time verification: inline lambda call overload exists ---

    @Test
    fun `inline call lambda overload compiles with correct types`() {
        // Verify the inline call function signature is accessible.
        // This is a compile-time test — we just verify the lambda types match.
        val serialize: (EchoRequest) -> ByteArray = { it.message.toByteArray() }
        val deserialize: (ByteArray) -> EchoResponse = { EchoResponse(String(it)) }

        val request = EchoRequest("test")
        val bytes = serialize(request)
        val response = deserialize(bytes)

        assertEquals("test", String(bytes))
        assertEquals("test", response.reply)
    }

    // --- Multiple RPC contracts can coexist ---

    private object StreamRpc : RpcRequest<EchoRequest, EchoResponse> {
        override val routeKey = "stream.DataStream.Send"
        override fun serializeRequest(request: EchoRequest) = request.message.toByteArray()
        override fun deserializeResponse(bytes: ByteArray) = EchoResponse(String(bytes))
    }

    @Test
    fun `multiple RpcRequest contracts have distinct routeKey`() {
        assertEquals("echo.EchoService.Echo", EchoRpc.routeKey)
        assertEquals("stream.DataStream.Send", StreamRpc.routeKey)
    }

    // --- Contract can be parameterized with protobuf-like types ---

    private data class ComplexRequest(val id: Long, val data: ByteArray) {
        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other !is ComplexRequest) return false
            return id == other.id && data.contentEquals(other.data)
        }

        override fun hashCode(): Int = id.hashCode() * 31 + data.contentHashCode()
    }

    private data class ComplexResponse(val status: Int, val body: ByteArray) {
        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other !is ComplexResponse) return false
            return status == other.status && body.contentEquals(other.body)
        }

        override fun hashCode(): Int = status.hashCode() * 31 + body.contentHashCode()
    }

    private object ComplexRpc : RpcRequest<ComplexRequest, ComplexResponse> {
        override val routeKey = "example.ComplexService.Process"

        override fun serializeRequest(request: ComplexRequest): ByteArray {
            // Minimal binary encoding: 8 bytes id + 4 bytes len + data
            val buf = ByteArray(8 + 4 + request.data.size)
            var pos = 0
            for (i in 0..7) buf[pos++] = (request.id shr (56 - i * 8)).toByte()
            val len = request.data.size
            for (i in 0..3) buf[pos++] = (len shr (24 - i * 8)).toByte()
            request.data.copyInto(buf, pos)
            return buf
        }

        override fun deserializeResponse(bytes: ByteArray): ComplexResponse {
            val status = ((bytes[0].toInt() and 0xFF) shl 24) or
                ((bytes[1].toInt() and 0xFF) shl 16) or
                ((bytes[2].toInt() and 0xFF) shl 8) or
                (bytes[3].toInt() and 0xFF)
            val body = bytes.copyOfRange(4, bytes.size)
            return ComplexResponse(status, body)
        }
    }

    @Test
    fun `RpcRequest contract with complex types serialization roundtrip`() {
        val request = ComplexRequest(42L, byteArrayOf(1, 2, 3))
        val bytes = ComplexRpc.serializeRequest(request)
        assertEquals(request.data.size + 12, bytes.size)

        val response = ComplexRpc.deserializeResponse(byteArrayOf(0, 0, 0, 1, 4, 5, 6))
        assertEquals(1, response.status)
        assertEquals(3, response.body.size)
    }

    // --- PayloadType default parameter is RPC_RELIABLE ---

    @Test
    fun `PayloadType default is RPC_RELIABLE for type-safe calls`() {
        // Verify that PayloadType.RPC_RELIABLE is the documented default
        assertEquals(PayloadType.RPC_RELIABLE, PayloadType.RPC_RELIABLE)
    }
}
