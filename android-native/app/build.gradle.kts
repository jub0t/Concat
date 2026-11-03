plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "app.concat.editor.spike"
    compileSdk = 35
    // The NDK the engine is built with; AGP strips the .so's symbols with it.
    ndkVersion = "28.2.13676358"

    defaultConfig {
        // Never app.concat.editor: this installs beside the shipping app.
        applicationId = "app.concat.editor.spike"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.0.1-spike"
        ndk { abiFilters += "arm64-v8a" }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // The debug key, so the spike's release build installs as-is.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true }

    packaging {
        // Compressed in the APK, as cargo-apk packs the Slint app's, so the
        // two sizes compare like for like. (AGP's default stores them
        // uncompressed for mmap, which is a bigger download.)
        jniLibs.useLegacyPackaging = true
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.core:core-ktx:1.15.0")
    // UniFFI's Kotlin calls the engine through JNA.
    implementation("net.java.dev.jna:jna:5.15.0@aar")
}
