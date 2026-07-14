package io.actrium.codegen

import com.google.protobuf.DescriptorProtos.MethodDescriptorProto
import com.google.protobuf.DescriptorProtos.DescriptorProto
import com.google.protobuf.DescriptorProtos.FileDescriptorProto
import com.google.protobuf.DescriptorProtos.FileOptions
import com.google.protobuf.DescriptorProtos.MethodOptions
import com.google.protobuf.DescriptorProtos.ServiceDescriptorProto
import com.google.protobuf.UnknownFieldSet
import com.google.protobuf.compiler.PluginProtos.CodeGeneratorRequest
import com.google.protobuf.compiler.PluginProtos.CodeGeneratorResponse
import kotlin.test.Test
import kotlin.test.assertContains
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse

class KotlinActorGeneratorTest {
    @Test
    fun generatedTypeReferencesAreQualifiedByOwnerOuterClass() {
        val methods =
                listOf(
                        MethodDescriptorProto.newBuilder()
                                .setName("Call")
                                .setInputType(".ask.Request")
                                .setOutputType(".reply.Response")
                                .build()
                )
        val typeOwner =
                mapOf(
                        "ask.Request" to TypeOwner("ask", "ask.proto"),
                        "reply.Response" to TypeOwner("reply", "reply.proto"),
                )

        val generated =
                KotlinActorGenerator(
                                packageName = "local",
                                serviceName = "LocalService",
                                methods = methods,
                                params = mapOf("kotlin_package" to "local.generated"),
                                protoFileName = "local.proto",
                                typeOwner = typeOwner,
                        )
                        .generate()
                        .content

        assertFalse(
                generated.contains("import ask.Ask.*"),
                "generated code should not wildcard-import owner classes:\n$generated",
        )
        assertContains(
                generated,
                "suspend fun call(request: ask.Ask.Request, ctx: ActrContext): reply.Reply.Response",
        )
        assertContains(generated, "val request = ask.Ask.Request.parseFrom(envelope.payload)")
    }

    @Test
    fun generatedRpcContractsIncludePayloadTypeAndCodecs() {
        val methods =
                listOf(
                        MethodDescriptorProto.newBuilder()
                                .setName("Echo")
                                .setInputType(".echo.EchoRequest")
                                .setOutputType(".echo.EchoResponse")
                                .build(),
                        MethodDescriptorProto.newBuilder()
                                .setName("Signal")
                                .setInputType(".echo.SignalRequest")
                                .setOutputType(".echo.SignalResponse")
                                .setOptions(payloadTypeOption(1))
                                .build(),
                )
        val typeOwner =
                mapOf(
                        "echo.EchoRequest" to TypeOwner("echo", "echo.proto"),
                        "echo.EchoResponse" to TypeOwner("echo", "echo.proto"),
                        "echo.SignalRequest" to TypeOwner("echo", "echo.proto"),
                        "echo.SignalResponse" to TypeOwner("echo", "echo.proto"),
                )

        val generated =
                KotlinActorGenerator(
                                packageName = "echo",
                                serviceName = "EchoService",
                                methods = methods,
                                params = mapOf("kotlin_package" to "echo.generated"),
                                protoFileName = "echo.proto",
                                typeOwner = typeOwner,
                        )
                        .generate()
                        .content

        assertContains(generated, "import io.actrium.actr.PayloadType")
        assertContains(generated, "import io.actrium.actr.dsl.RpcRequest")
        assertContains(
                generated,
                "object EchoEchoRpc : RpcRequest<echo.Echo.EchoRequest, echo.Echo.EchoResponse>",
        )
        assertContains(generated, "override val routeKey: String = \"echo.EchoService.Echo\"")
        assertContains(generated, "override val payloadType: PayloadType = PayloadType.RPC_RELIABLE")
        assertContains(
                generated,
                "override fun serializeRequest(request: echo.Echo.EchoRequest): ByteArray = request.toByteArray()",
        )
        assertContains(
                generated,
                "override fun deserializeResponse(bytes: ByteArray): echo.Echo.EchoResponse = echo.Echo.EchoResponse.parseFrom(bytes)",
        )
        assertContains(
                generated,
                "object EchoSignalRpc : RpcRequest<echo.Echo.SignalRequest, echo.Echo.SignalResponse>",
        )
        assertContains(generated, "override val payloadType: PayloadType = PayloadType.RPC_SIGNAL")
    }

    @Test
    fun generatedRpcContractNamesDoNotCollideWhenMethodStartsWithServiceBase() {
        val methods =
                listOf(
                        MethodDescriptorProto.newBuilder()
                                .setName("Bar")
                                .setInputType(".echo.BarRequest")
                                .setOutputType(".echo.BarResponse")
                                .build(),
                        MethodDescriptorProto.newBuilder()
                                .setName("EchoBar")
                                .setInputType(".echo.EchoBarRequest")
                                .setOutputType(".echo.EchoBarResponse")
                                .build(),
                )
        val typeOwner =
                mapOf(
                        "echo.BarRequest" to TypeOwner("echo", "echo.proto"),
                        "echo.BarResponse" to TypeOwner("echo", "echo.proto"),
                        "echo.EchoBarRequest" to TypeOwner("echo", "echo.proto"),
                        "echo.EchoBarResponse" to TypeOwner("echo", "echo.proto"),
                )

        val generated =
                KotlinActorGenerator(
                                packageName = "echo",
                                serviceName = "EchoService",
                                methods = methods,
                                params = mapOf("kotlin_package" to "echo.generated"),
                                protoFileName = "echo.proto",
                                typeOwner = typeOwner,
                        )
                        .generate()
                        .content

        assertContains(
                generated,
                "object EchoBarRpc : RpcRequest<echo.Echo.BarRequest, echo.Echo.BarResponse>",
        )
        assertContains(
                generated,
                "object EchoEchoBarRpc : RpcRequest<echo.Echo.EchoBarRequest, echo.Echo.EchoBarResponse>",
        )
    }

    @Test
    fun unknownPayloadTypeAnnotationFailsCodegen() {
        val methods =
                listOf(
                        MethodDescriptorProto.newBuilder()
                                .setName("Stream")
                                .setInputType(".echo.StreamRequest")
                                .setOutputType(".echo.StreamResponse")
                                .setServerStreaming(true)
                                .setOptions(payloadTypeOption(5))
                                .build()
                )
        val typeOwner =
                mapOf(
                        "echo.StreamRequest" to TypeOwner("echo", "echo.proto"),
                        "echo.StreamResponse" to TypeOwner("echo", "echo.proto"),
                )

        val error =
                assertFailsWith<IllegalArgumentException> {
                    KotlinActorGenerator(
                                    packageName = "echo",
                                    serviceName = "EchoService",
                                    methods = methods,
                                    params = mapOf("kotlin_package" to "echo.generated"),
                                    protoFileName = "echo.proto",
                                    typeOwner = typeOwner,
                            )
                            .generate()
                }

        assertContains(error.message.orEmpty(), "Unsupported (actr.payload_type) value 5")
    }

    @Test
    fun generateCodeQualifiesImportedAskRpcTypesForDataStreamApp() {
        val request =
                CodeGeneratorRequest.newBuilder()
                        .addFileToGenerate("data_stream_app.proto")
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("ask.proto")
                                        .setPackage("ask")
                                        .addMessageType(
                                                DescriptorProto.newBuilder()
                                                        .setName(
                                                                "ContinuePromptResultStreamsRequest"))
                                        .addMessageType(
                                                DescriptorProto.newBuilder()
                                                        .setName(
                                                                "ContinuePromptResultStreamsResponse"))
                        )
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("data_stream_app.proto")
                                        .setPackage("data_stream_app")
                                        .addService(
                                                ServiceDescriptorProto.newBuilder()
                                                        .setName("DataStreamApp")
                                                        .addMethod(
                                                                MethodDescriptorProto.newBuilder()
                                                                        .setName(
                                                                                "ContinuePromptResultStreams")
                                                                        .setInputType(
                                                                                ".ask.ContinuePromptResultStreamsRequest")
                                                                        .setOutputType(
                                                                                ".ask.ContinuePromptResultStreamsResponse")
                                                        )
                                        )
                        )
                        .build()

        val generated = actorContent(generateCode(request))

        assertContains(
                generated,
                "suspend fun continue_prompt_result_streams(request: ask.Ask.ContinuePromptResultStreamsRequest, ctx: ActrContext): ask.Ask.ContinuePromptResultStreamsResponse",
        )
        assertContains(
                generated,
                "val request = ask.Ask.ContinuePromptResultStreamsRequest.parseFrom(envelope.payload)",
        )
        assertFalse(
                generated.contains("request: ContinuePromptResultStreamsRequest"),
                "generated code should keep the imported ask owner:\n$generated",
        )
    }

    @Test
    fun generateCodeHonorsJavaPackageOuterClassAndMultipleFiles() {
        val request =
                CodeGeneratorRequest.newBuilder()
                        .addFileToGenerate("local.proto")
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("user_types.proto")
                                        .setPackage("proto.user.v1")
                                        .setOptions(
                                                FileOptions.newBuilder()
                                                        .setJavaPackage("com.example.user.v1")
                                                        .setJavaOuterClassname("UserTypesProto")
                                        )
                                        .addMessageType(
                                                DescriptorProto.newBuilder().setName("Request"))
                        )
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("response_types.proto")
                                        .setPackage("proto.response.v1")
                                        .setOptions(
                                                FileOptions.newBuilder()
                                                        .setJavaPackage("com.example.response.v1")
                                                        .setJavaMultipleFiles(true)
                                        )
                                        .addMessageType(
                                                DescriptorProto.newBuilder().setName("Response"))
                        )
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("local.proto")
                                        .setPackage("local")
                                        .addService(
                                                ServiceDescriptorProto.newBuilder()
                                                        .setName("LocalService")
                                                        .addMethod(
                                                                MethodDescriptorProto.newBuilder()
                                                                        .setName("Call")
                                                                        .setInputType(
                                                                                ".proto.user.v1.Request")
                                                                        .setOutputType(
                                                                                ".proto.response.v1.Response")
                                                        )
                                        )
                        )
                        .build()

        val response = generateCode(request)
        val generated = actorContent(response)
        val metadata = response.fileList.single { it.name == "actr-gen-meta.json" }.content

        assertContains(generated, "request: com.example.user.v1.UserTypesProto.Request")
        assertContains(generated, "): com.example.response.v1.Response")
        assertContains(
                generated,
                "val request = com.example.user.v1.UserTypesProto.Request.parseFrom(envelope.payload)",
        )
        assertContains(
                metadata,
                "\"generated_type\": \"com.example.user.v1.UserTypesProto.Request\"",
        )
        assertContains(
                metadata,
                "\"generated_type\": \"com.example.response.v1.Response\"",
        )
    }

    @Test
    fun generateCodeUsesOuterClassSuffixWhenDefaultOuterClassCollides() {
        val request =
                CodeGeneratorRequest.newBuilder()
                        .addFileToGenerate("local.proto")
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("request.proto")
                                        .setPackage("proto.request.v1")
                                        .setOptions(
                                                FileOptions.newBuilder()
                                                        .setJavaPackage("com.example.request.v1")
                                        )
                                        .addMessageType(
                                                DescriptorProto.newBuilder().setName("Request"))
                        )
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("local.proto")
                                        .setPackage("local")
                                        .addService(
                                                ServiceDescriptorProto.newBuilder()
                                                        .setName("LocalService")
                                                        .addMethod(
                                                                MethodDescriptorProto.newBuilder()
                                                                        .setName("Call")
                                                                        .setInputType(
                                                                                ".proto.request.v1.Request")
                                                                        .setOutputType(
                                                                                ".proto.request.v1.Request")
                                                        )
                                        )
                        )
                        .build()

        val generated = actorContent(generateCode(request))

        assertContains(generated, "com.example.request.v1.RequestOuterClass.Request")
    }

    @Test
    fun generateCodeErrorsOnUnresolvedUnqualifiedRpcType() {
        val request =
                CodeGeneratorRequest.newBuilder()
                        .addFileToGenerate("local.proto")
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("local.proto")
                                        .setPackage("local")
                                        .addService(
                                                ServiceDescriptorProto.newBuilder()
                                                        .setName("LocalService")
                                                        .addMethod(
                                                                MethodDescriptorProto.newBuilder()
                                                                        .setName("Call")
                                                                        .setInputType("MissingRequest")
                                                                        .setOutputType("MissingResponse")
                                                        )
                                        )
                        )
                        .build()

        val error = assertFailsWith<IllegalArgumentException> { generateCode(request) }

        assertContains(error.message ?: "", "Cannot resolve input type `MissingRequest`")
    }

    @Test
    fun generateCodeWritesMetadataWithDescriptorOwnerRefs() {
        val request =
                CodeGeneratorRequest.newBuilder()
                        .addFileToGenerate("ask.proto")
                        .addFileToGenerate("local.proto")
                        .setParameter("LocalFiles=local.proto,RemoteFiles=ask.proto,RemoteFileMapping=ask.proto=acme:Ask:1.0.0")
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("ask.proto")
                                        .setPackage("ask")
                                        .addMessageType(
                                                DescriptorProto.newBuilder()
                                                        .setName("Request"))
                                        .addMessageType(
                                                DescriptorProto.newBuilder()
                                                        .setName("Response"))
                                        .addService(
                                                ServiceDescriptorProto.newBuilder()
                                                        .setName("AskService")
                                                        .addMethod(
                                                                MethodDescriptorProto.newBuilder()
                                                                        .setName("Ask")
                                                                        .setInputType(".ask.Request")
                                                                        .setOutputType(".ask.Response")
                                                        )
                                        )
                        )
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("local.proto")
                                        .setPackage("local")
                                        .addService(
                                                ServiceDescriptorProto.newBuilder()
                                                        .setName("LocalService")
                                                        .addMethod(
                                                                MethodDescriptorProto.newBuilder()
                                                                        .setName("Call")
                                                                        .setInputType(".ask.Request")
                                                                        .setOutputType(".ask.Response")
                                                        )
                                        )
                        )
                        .build()

        val metadata =
                generateCode(request).fileList.single { it.name == "actr-gen-meta.json" }.content

        assertContains(metadata, "\"language\": \"kotlin\"")
        assertContains(metadata, "\"local_services\"")
        assertContains(metadata, "\"remote_services\"")
        assertContains(metadata, "\"actr_type\": \"acme:Ask:1.0.0\"")
        assertContains(metadata, "\"input_ref\": {\"proto_type\": \"ask.Request\"")
        assertContains(metadata, "\"proto_package\": \"ask\"")
        assertContains(metadata, "\"proto_file\": \"ask.proto\"")
    }

    @Test
    fun generateCodeErrorsOnUnresolvedQualifiedRpcType() {
        val request =
                CodeGeneratorRequest.newBuilder()
                        .addFileToGenerate("local.proto")
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName("local.proto")
                                        .setPackage("local")
                                        .addService(
                                                ServiceDescriptorProto.newBuilder()
                                                        .setName("LocalService")
                                                        .addMethod(
                                                                MethodDescriptorProto.newBuilder()
                                                                        .setName("Call")
                                                                        .setInputType(
                                                                                ".external.MissingRequest")
                                                                        .setOutputType(
                                                                                ".external.MissingResponse")
                                                        )
                                        )
                        )
                        .build()

        val error = assertFailsWith<IllegalArgumentException> { generateCode(request) }

        assertContains(error.message ?: "", "Cannot resolve input type `external.MissingRequest`")
    }

    @Test
    fun metadataUsesCanonicalMethodNamesAndProtoPaths() {
        val request =
                CodeGeneratorRequest.newBuilder()
                        .addFileToGenerate(".\\remote\\ask")
                        .setParameter(
                                "RemoteFiles=./remote/ask,RemoteFileMapping=./remote/ask=acme:Ask:1.0.0")
                        .addProtoFile(
                                FileDescriptorProto.newBuilder()
                                        .setName(".\\remote\\ask")
                                        .setPackage("ask")
                                        .addMessageType(
                                                DescriptorProto.newBuilder().setName("Request"))
                                        .addMessageType(
                                                DescriptorProto.newBuilder().setName("Response"))
                                        .addService(
                                                ServiceDescriptorProto.newBuilder()
                                                        .setName("AskService")
                                                        .addMethod(
                                                                MethodDescriptorProto.newBuilder()
                                                                        .setName("HTTPServer")
                                                                        .setInputType(".ask.Request")
                                                                        .setOutputType(".ask.Response")
                                                        )
                                        )
                        )
                        .build()

        val response = generateCode(request)
        val metadata = response.fileList.single { it.name == "actr-gen-meta.json" }.content

        assertContains(metadata, "\"proto_file\": \"remote/ask.proto\"")
        assertContains(metadata, "\"snake_name\": \"http_server\"")
        assertContains(actorContent(response), "suspend fun http_server(")
    }
}

private fun actorContent(response: CodeGeneratorResponse): String {
    return response.fileList.single { it.name != "actr-gen-meta.json" }.content
}

private fun payloadTypeOption(value: Long): MethodOptions {
    val field =
            UnknownFieldSet.Field.newBuilder()
                    .addVarint(value)
                    .build()
    val unknownFields =
            UnknownFieldSet.newBuilder()
                    .addField(50001, field)
                    .build()
    return MethodOptions.newBuilder().setUnknownFields(unknownFields).build()
}
