pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        // Resolve the freshly-built local AAR (./gradlew :actr-kotlin:publishToMavenLocal).
        mavenLocal()
        google()
        mavenCentral()
    }
}

rootProject.name = "__PROJECT_NAME__"

include(":app")
