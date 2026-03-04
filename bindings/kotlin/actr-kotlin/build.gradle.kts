plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
    id("maven-publish")
}

group = "io.actorrtc"

version = "0.1.0"

android {
    namespace = "io.actorrtc.actr"
    compileSdk = 34

    defaultConfig {
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    // Suppress lint errors for UniFFI generated code
    lint {
        abortOnError = false
        disable += listOf("NewApi")
    }
}

dependencies {
    // Kotlin standard library
    implementation("org.jetbrains.kotlin:kotlin-stdlib:1.9.22")

    // Coroutines
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.0")

    // JNA for UniFFI bindings - use Android-compatible version
    implementation("net.java.dev.jna:jna:5.14.0@aar")

    // Testing
    testImplementation(kotlin("test"))
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.8.0")
}

// For publishing to local maven
publishing {
    publications {
        create<MavenPublication>("maven") {
            afterEvaluate {
                from(components["release"])
            }

            pom {
                name.set("actr-kotlin")
                description.set("Kotlin bindings for the Actor-RTC framework")
                url.set("https://github.com/actor-rtc/actr-kotlin")

                licenses {
                    license {
                        name.set("Apache-2.0")
                        url.set("https://www.apache.org/licenses/LICENSE-2.0")
                    }
                }
            }
        }
    }
}
