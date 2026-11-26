plugins {
    id("com.android.application")
}

android {
    namespace = "dev.launcherrs.gallery"
    compileSdk = 37
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "dev.launcherrs.gallery"
        minSdk = 26
        targetSdk = 36
        versionCode = 2
        versionName = "0.1.1"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        release {
            // Shrink the Java side; keep rules protect what Rust calls by name.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Signed with the local debug key, like the launcher.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    // HEIC pictures through the phone's HEVC encoder (Apache 2.0).
    implementation("androidx.heifwriter:heifwriter:1.0.0")
}
