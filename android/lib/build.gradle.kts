// The AAR: Uriel's Rust client for Android, and the Kotlin bindings that UniFFI generates for it.
// Gradle builds the Rust library with cargo-ndk, and generates the bindings from it, so the two
// always match.

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

// The ABIs the AAR ships. iceoryx2, which Uriel is built on, does not support 32-bit Android.
val abis = listOf("arm64-v8a", "x86_64")
val ndkVersionName = "30.0.16248370"
// The oldest Android that the AAR supports (Android 8).
val minSdkVersion = 26

val rustDir = layout.projectDirectory.dir("../uniffi")
val rustOutput = layout.buildDirectory.dir("rustLibs")
val jniLibs = layout.buildDirectory.dir("jniLibs")
val generatedKotlin = layout.buildDirectory.dir("generated/uniffi")
// The library that the bindings are generated from. The bindings are the same for every ABI.
val bindingsSource = "liburiel_android.so"

android {
    namespace = "org.cmusatyalab.uriel"
    compileSdk = 34
    ndkVersion = ndkVersionName

    defaultConfig {
        minSdk = minSdkVersion
        consumerProguardFiles("consumer-rules.pro")
        ndk { abiFilters += abis }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }

    sourceSets["main"].java.srcDir(generatedKotlin)
    sourceSets["main"].jniLibs.srcDir(jniLibs)
}

// UniFFI's Kotlin calls the Rust library through JNA, which has to ship as an AAR to bring its
// own native library.
dependencies {
    api("net.java.dev.jna:jna:5.17.0@aar")
}

val androidComponents = extensions.getByType<com.android.build.api.variant.LibraryAndroidComponentsExtension>()

val buildRust by tasks.registering(Exec::class) {
    description = "Builds the Rust library for each ABI with cargo-ndk."
    workingDir = rustDir.asFile
    doFirst {
        val sdk = androidComponents.sdkComponents.sdkDirectory.get()
        environment("ANDROID_NDK_HOME", sdk.dir("ndk/$ndkVersionName").asFile)
        commandLine(
            listOf("cargo", "ndk") +
                abis.flatMap { listOf("-t", it) } +
                listOf("-P", "$minSdkVersion", "-o", rustOutput.get().asFile.absolutePath) +
                listOf("build", "--release"),
        )
    }
}

val copyJniLibs by tasks.registering(Sync::class) {
    description = "Copies the Rust library, and nothing cargo-ndk also leaves, into jniLibs."
    dependsOn(buildRust)
    from(rustOutput) { include("*/liburiel_android.so") }
    into(jniLibs)
}

val generateBindings by tasks.registering(Exec::class) {
    description = "Generates the Kotlin bindings from the Rust library."
    dependsOn(buildRust)
    workingDir = rustDir.asFile
    doFirst {
        delete(generatedKotlin)
        commandLine(
            "cargo", "run", "--quiet", "--features", "cli", "--bin", "uniffi-bindgen", "--",
            "generate",
            "--library", rustOutput.get().file("${abis.first()}/$bindingsSource").asFile.absolutePath,
            "--language", "kotlin",
            "--no-format",
            "--out-dir", generatedKotlin.get().asFile.absolutePath,
        )
    }
}

tasks.named("preBuild") { dependsOn(copyJniLibs, generateBindings) }
