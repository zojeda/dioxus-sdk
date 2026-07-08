// Kotlin Multiplatform push-notification bridge for `dioxus-sdk-push`.
//
// This module is consumed in two ways:
//
// 1. Auto-bundled by `dx` (>= 0.7.9): the Rust crate embeds plugin metadata via
//    `#[manganis::ffi]`, and `dx` copies this folder into the generated Android
//    project as `:plugins:dioxuspushkotlin`. In that mode `dx` strips every
//    `version "..."` suffix below, because the parent project already provides the
//    plugins on its buildscript classpath (AGP 8.7.0 / Kotlin 2.0.20).
// 2. Standalone, as a regular KMP library. The plugin versions below apply and the
//    iOS targets can be enabled with `-Pdev.dioxus.push.ios=true`.

plugins {
    id("org.jetbrains.kotlin.multiplatform") version "2.0.20"
    id("com.android.library") version "8.7.0"
}

kotlin {
    androidTarget {
        compilations.all {
            kotlinOptions.jvmTarget = "17"
        }
    }

    // Kotlin/Native iOS targets are opt-in so the dx-embedded Android build never
    // configures (or downloads) Kotlin/Native toolchains. On Dioxus iOS the Rust
    // crate talks to APNs natively; iosMain exists for pure-Kotlin/KMP consumers.
    if (providers.gradleProperty("dev.dioxus.push.ios").isPresent) {
        iosArm64()
        iosX64()
        iosSimulatorArm64()
    }

    sourceSets {
        // commonMain has no dependencies: the shared surface is pure Kotlin.
        // iosMain is created by the default hierarchy template when the iOS
        // targets above are enabled; it needs no extra dependencies either.
        androidMain.dependencies {
            api("com.google.firebase:firebase-messaging:24.1.0")
            implementation("androidx.core:core-ktx:1.13.1")
        }
    }
}

android {
    namespace = "dev.dioxus.push"
    compileSdk = 34

    defaultConfig {
        minSdk = 24
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
