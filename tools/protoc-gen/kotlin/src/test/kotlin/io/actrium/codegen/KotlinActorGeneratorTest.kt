package io.actrium.codegen

import com.google.protobuf.DescriptorProtos.MethodDescriptorProto
import com.google.protobuf.DescriptorProtos.DescriptorProto
import com.google.protobuf.DescriptorProtos.FileDescriptorProto
import com.google.protobuf.DescriptorProtos.FileOptions
import com.google.protobuf.DescriptorProtos.ServiceDescriptorProto
import com.google.protobuf.compiler.PluginProtos.CodeGeneratorRequest
import kotlin.test.Test
import kotlin.test.assertContains
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

        val generated = generateCode(request).fileList.single().content

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

        val generated = generateCode(request).fileList.single().content

        assertContains(generated, "request: com.example.user.v1.UserTypesProto.Request")
        assertContains(generated, "): com.example.response.v1.Response")
        assertContains(
                generated,
                "val request = com.example.user.v1.UserTypesProto.Request.parseFrom(envelope.payload)",
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

        val generated = generateCode(request).fileList.single().content

        assertContains(generated, "com.example.request.v1.RequestOuterClass.Request")
    }
}
