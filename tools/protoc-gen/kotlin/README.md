# framework-codegen-kotlin

[![Kotlin](https://img.shields.io/badge/Kotlin-1.9.22-blue.svg)](https://kotlinlang.org/)
[![Gradle](https://img.shields.io/badge/Gradle-8.5-green.svg)](https://gradle.org/)
[![Protobuf](https://img.shields.io/badge/Protobuf-3.25.1-orange.svg)](https://protobuf.dev/)

A Kotlin protoc plugin for generating ACTR framework code from Protocol Buffer service definitions.

## Overview

This project implements a protoc plugin (`protoc-gen-actrframework-kotlin`) that generates Kotlin code for the ACTR (Actor-RTC) framework. It generates:

- **Handler Interfaces**: User-implemented business logic interfaces
- **Dispatcher Objects**: Zero-overhead message routing objects

The plugin is designed to work alongside the protobuf Gradle plugin, which handles message class generation.

## Architecture

This project mirrors the Rust implementation (`framework-protoc-codegen`) but generates Kotlin code instead:

```
┌─────────────────┐    ┌──────────────────────┐    ┌─────────────────┐
│   actr-cli      │────│ protoc-gen-actrframe │────│   Generated     │
│                 │    │ work-kotlin          │    │   Kotlin Code   │
│ - Command line  │    │                      │    │                 │
│ - Language      │    │ - Handler interfaces │    │ - Handlers      │
│   selection     │    │ - Dispatcher objects │    │ - Dispatchers   │
└─────────────────┘    └──────────────────────┘    └─────────────────┘
                              │
                              ▼
                   ┌──────────────────────┐    ┌─────────────────┐
                   │ protobuf-gradle      │────│   Generated     │
                   │ plugin               │    │   Message       │
                   │                      │    │   Classes       │
                   │ - Message classes    │    │                 │
                   │ - Builders           │    │ - Builders      │
                   │ - Parsers            │    │ - Parsers       │
                   └──────────────────────┘    └─────────────────┘
```

## Generated Code Example

For a proto service like:

```proto
service LocalFileService {
    rpc SendFile(SendFileRequest) returns (SendFileResponse);
}
```

The plugin generates:

```kotlin
// Handler interface (user implements this)
interface LocalFileServiceHandler {
    suspend fun send_file(request: SendFileRequest, ctx: ContextBridge): SendFileResponse
}

// Dispatcher object (zero-overhead routing)
object LocalFileServiceDispatcher {
    suspend fun dispatch(
        handler: LocalFileServiceHandler,
        ctx: ContextBridge,
        envelope: RpcEnvelopeBridge
    ): ByteArray {
        return when (envelope.routeKey) {
            "local_file.LocalFileService.SendFile" -> {
                val request = SendFileRequest.parseFrom(envelope.payload)
                val response = handler.send_file(request, ctx)
                response.toByteArray()
            }
            else -> throw IllegalArgumentException("Unknown route key: ${envelope.routeKey}")
        }
    }
}
```

## Usage

### As a protoc Plugin

```bash
# Build the plugin
./gradlew protocPluginJar

# Use with protoc
protoc --plugin=protoc-gen-actrframework-kotlin=./protoc-gen-actrframework-kotlin \
       --actrframework-kotlin_out=output_dir \
       --actrframework-kotlin_opt=kotlin_package=com.example.generated \
       input.proto
```

### With actr-cli

The plugin is automatically used by `actr-cli` when generating Kotlin code:

```bash
actr gen --input=proto/ --output=src/main/java/com/example/generated --language=kotlin
```

## Building

### Prerequisites

- JDK 17 or higher
- Gradle 8.5 or higher

### Build Commands

```bash
# Build the project
./gradlew build

# Build the protoc plugin JAR
./gradlew protocPluginJar

# Run tests
./gradlew test

# Clean build
./gradlew clean
```

### Generated Artifacts

- `build/libs/framework-codegen-kotlin-0.1.0.jar` - Main JAR
- `build/libs/protoc-gen-actrframework-kotlin-0.1.0.jar` - Protoc plugin JAR
- `protoc-gen-actrframework-kotlin` - Executable wrapper script

## Project Structure

```
framework-codegen-kotlin/
├── src/main/kotlin/io/actor_rtc/codegen/
│   ├── Main.kt                    # Protoc plugin entry point
│   └── KotlinActorGenerator.kt    # Core code generator
├── build.gradle.kts               # Gradle build configuration
├── settings.gradle.kts            # Gradle settings
├── protoc-gen-actrframework-kotlin # Executable wrapper
└── README.md                      # This file
```

## Dependencies

- `com.google.protobuf:protobuf-java:3.25.1` - Protobuf runtime
- `org.jetbrains.kotlin:kotlin-stdlib` - Kotlin standard library

## Development

### Adding New Features

1. Modify `KotlinActorGenerator.kt` to add new generation logic
2. Update tests in `src/test/kotlin/`
3. Run `./gradlew test` to verify changes
4. Update this README if needed

### Testing

```bash
# Run unit tests
./gradlew test

# Run with coverage
./gradlew test jacocoTestReport
```

## Integration with ACTR Framework

This plugin is part of the ACTR (Actor-RTC) ecosystem:

- **actr-cli**: Command-line tool that uses this plugin
- **framework-protoc-codegen**: Rust equivalent for Rust code generation
- **actr-framework**: Core framework runtime
- **actr-protocol**: Protocol definitions

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Submit a pull request

## License

Licensed under the Apache License, Version 2.0. See LICENSE file for details.

## Version History

- **0.1.0**: Initial release
  - Basic Handler interface generation
  - Dispatcher object generation
  - Protoc plugin integration
  - actr-cli integration

---

**Generated by**: protoc-gen-actrframework-kotlin
**Framework**: ACTR (Actor-RTC)
**Language**: Kotlin