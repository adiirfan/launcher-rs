plugins {
    id("com.android.application")
}

android {
    namespace = "dev.launcherrs.app"
    compileSdk = 37
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "dev.launcherrs.app"
        minSdk = 26
        targetSdk = 36
        versionCode = 6
        versionName = "0.1.5"
        ndk { abiFilters += "arm64-v8a" }
    }

    buildTypes {
        release {
            // Shrink the Java side; keep rules protect what Rust calls by name.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Signed with the local debug key, for testing on our phones.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

