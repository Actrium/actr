plugins {
    id("com.android.application") version "8.12.2" apply false
    id("com.android.library") version "8.12.2" apply false
    id("org.jetbrains.kotlin.android") version "1.9.22" apply false
    id("org.jetbrains.kotlinx.kover") version "0.9.1" apply false
    id("com.google.protobuf") version "0.9.4" apply false
    id("org.jlleitschuh.gradle.ktlint") version "12.1.2" apply false
}

// ktlint: only lint demo code (exclude library bridge/generated code)
project(":demo") {
    apply(plugin = "org.jlleitschuh.gradle.ktlint")

    configure<org.jlleitschuh.gradle.ktlint.KtlintExtension> {
        version.set("1.5.0")
        android.set(true)
    }
}
