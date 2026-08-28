pluginManagement {
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
        gradlePluginPortal()
        // Mihomo Android core (AAR / JNI) is published on JitPack by MetaCubeX.
        maven { url = uri("https://jitpack.io") }
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // Mihomo Android core (AAR / JNI).
        maven { url = uri("https://jitpack.io") }
    }
}

rootProject.name = "ClashEdge"
include(":app")
