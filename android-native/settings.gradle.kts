// The spike shell: a native Kotlin + Jetpack Compose app over the Rust
// engine (src/crates/concat-mobile). Its own application id, so it installs
// beside the shipping Slint app and never replaces it.
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
    }
}
rootProject.name = "concat-spike"
include(":app")
