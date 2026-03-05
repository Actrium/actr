plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("com.google.protobuf")
}

android {
    namespace = "{{PACKAGE_NAME}}"
    compileSdk = 34

    defaultConfig {
        applicationId = "{{PACKAGE_NAME}}"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "1.0.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                    getDefaultProguardFile("proguard-android-optimize.txt"),
                    "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions { jvmTarget = "17" }

    buildFeatures { viewBinding = true }
}

// Protobuf configuration for message generation
protobuf {
    protoc { artifact = "com.google.protobuf:protoc:3.25.1" }
    generateProtoTasks {
        all().forEach { task -> task.builtins { create("java") { option("lite") } } }
    }
}

// Copy proto files from protos/remote and protos/local to src/main/proto for protobuf plugin
val copyProtos =
        tasks.register<Copy>("copyProtos") {
            from("${rootProject.projectDir}/protos/remote") { include("**/*.proto") }
            from("${rootProject.projectDir}/protos/local") { include("**/*.proto") }
            into("src/main/proto")
        }

// Copy Actr.lock.toml to assets for runtime service resolution
val copyLockFile =
        tasks.register<Copy>("copyLockFile") {
            from("${rootProject.projectDir}/Actr.lock.toml")
            into("src/main/assets")
        }

// Copy Actr.toml to assets for runtime configuration
val copyActrConfig =
        tasks.register<Copy>("copyActrConfig") {
            from("${rootProject.projectDir}/Actr.toml")
            into("src/main/assets")
        }

// Make proto generation depend on copyProtos
afterEvaluate {
    tasks
            .matching { it.name.startsWith("generateProto") || it.name.startsWith("extractProto") }
            .configureEach { dependsOn(copyProtos) }
    // Make compile tasks depend on copyLockFile and copyActrConfig
    tasks.matching { it.name.startsWith("merge") && it.name.contains("Assets") }.configureEach {
        dependsOn(copyLockFile)
        dependsOn(copyActrConfig)
    }
}

dependencies {
    // actr-kotlin library from JitPack
    implementation("com.github.actor-rtc:actr-kotlin:5ae5676") {
        exclude(group = "net.java.dev.jna", module = "jna")
    }

    // JNA for UniFFI bindings (required by actr-kotlin)
    implementation("net.java.dev.jna:jna:5.14.0@aar")

    // Protobuf runtime
    implementation("com.google.protobuf:protobuf-javalite:3.25.1")

    // Android core
    implementation("androidx.core:core-ktx:1.12.0")
    implementation("androidx.appcompat:appcompat:1.6.1")
    implementation("com.google.android.material:material:1.11.0")
    implementation("androidx.constraintlayout:constraintlayout:2.1.4")

    // Lifecycle
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.7.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-ktx:2.7.0")

    // Coroutines
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.0")

    // Testing
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.1")
}
    // Lifecycle
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.7.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-ktx:2.7.0")

    // Coroutines
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.0")

    // Testing
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.1")
}
