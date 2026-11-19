plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    // Builds the Rust crate for every ABI and build type, links the istmo
    // plugins it depends on (Kotlin sources, manifests, resources, Gradle
    // dependencies) and applies istmo.toml `[app]` (applicationId,
    // version, minSdk, label, icon).
    id("io.github.sergioribera.istmo")
}

// `-Pistmo.abis=arm64-v8a,x86_64` (the release workflow, from nix/targets.nix)
// → one APK per ABI plus a universal one. Without it the local flow
// builds the `abiFilters` below into a single APK.
val releaseAbis = (findProperty("istmo.abis") as String?)
    ?.split(',')
    ?.map { it.trim() }
    ?.filter { it.isNotEmpty() }

// With splits on, ndk.abiFilters stays empty and the istmo plugin falls
// back to `defaultAbis` for the cargo builds.
releaseAbis?.let { abis -> istmo { defaultAbis.set(abis) } }

android {
    namespace  = "rs.sergioribera.myos"
    compileSdk = 34

    defaultConfig {
        targetSdk = 34
        // AGP rejects ndk.abiFilters together with splits.abi.
        if (releaseAbis == null) ndk { abiFilters += setOf("arm64-v8a", "x86_64") }
    }

    splits {
        abi {
            isEnable = releaseAbis != null
            reset()
            releaseAbis?.let { include(*it.toTypedArray()) }
            isUniversalApk = true
        }
    }

    signingConfigs {
        getByName("debug") {
            storeFile     = file(System.getenv("ANDROID_DEBUG_KEYSTORE")
                ?: "${System.getProperty("user.home")}/.android/debug.keystore")
            storePassword = "android"
            keyAlias      = "androiddebugkey"
            keyPassword   = "android"
        }
        // Release signing (GitHub release APKs, Play Store bundle): CI writes
        // the keystore from secrets. Without it release builds stay unsigned.
        System.getenv("ANDROID_RELEASE_KEYSTORE")?.let { keystorePath ->
            create("release") {
                storeFile     = file(keystorePath)
                storePassword = System.getenv("ANDROID_RELEASE_STORE_PASSWORD")
                keyAlias      = System.getenv("ANDROID_RELEASE_KEY_ALIAS")
                keyPassword   = System.getenv("ANDROID_RELEASE_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        getByName("debug") { signingConfig = signingConfigs.getByName("debug") }
        getByName("release") {
            isMinifyEnabled = false
            signingConfigs.findByName("release")?.let { signingConfig = it }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    packaging { jniLibs { useLegacyPackaging = false } }
}

dependencies {
    // Kotlin runtime; shares its version with the istmo crates and the Gradle plugin.
    implementation("io.github.sergioribera:istmo-runtime:0.1.1-alpha.6")
    // `IstmoGameActivity` extends GameActivity.
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")
}
