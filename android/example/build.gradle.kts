// An example app, in Java, that sends inputs to a remote Uriel server and shows what happens to
// them. See README.md for how to run it.

plugins {
    id("com.android.application")
}

android {
    namespace = "org.cmusatyalab.uriel.example"
    compileSdk = 34

    defaultConfig {
        applicationId = "org.cmusatyalab.uriel.example"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":lib"))
    implementation("androidx.appcompat:appcompat:1.7.0")
}
