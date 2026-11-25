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
        google()
        mavenCentral()
        // The Kotlin half of rustls-platform-verifier (Android certificate checks for the
        // Rust core, issue #243). Its maintainers publish it only here. Limited to its own
        // group, so nothing else can ever be fetched from this repository.
        exclusiveContent {
            forRepository {
                maven {
                    url = uri(
                        "https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/",
                    )
                }
            }
            filter { includeGroup("org.rustls") }
        }
    }
}

rootProject.name = "mailcrane"
include(":app")
