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
        mavenLocal()
        google()
        mavenCentral()
        // actr-kotlin is published to GitHub Packages; consuming it requires
        // authentication. Set `gpr.user` / `gpr.key` in ~/.gradle/gradle.properties
        // (token needs `read:packages`), or rely on GITHUB_ACTOR / GITHUB_TOKEN in CI.
        // For local dev without credentials, run
        //   ./gradlew :actr-kotlin:publishToMavenLocal -PactrGroup=io.actrium -PactrVersion=0.4.11
        // and mavenLocal() above will satisfy the dependency.
        maven {
            url = uri("https://maven.pkg.github.com/Actrium/actr-kotlin-package-sync")
            credentials {
                username = providers.gradleProperty("gpr.user")
                    .orElse(providers.environmentVariable("GITHUB_ACTOR"))
                    .orNull
                password = providers.gradleProperty("gpr.key")
                    .orElse(providers.environmentVariable("GITHUB_TOKEN"))
                    .orNull
            }
        }
    }
}

rootProject.name = "actr-kotlin"

include(":actr-kotlin")
if (file("demo").isDirectory) {
    include(":demo")
}
