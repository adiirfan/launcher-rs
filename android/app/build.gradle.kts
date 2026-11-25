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
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += "arm64-v8a" }
    }

    buildTypes {
        release { isMinifyEnabled = false }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    // Must match what android-activity 0.6.1 was built against. No prefab.
    implementation("androidx.games:games-activity:4.4.0")
    // GameActivity extends AppCompatActivity but declares no dependencies of its own.
    implementation("androidx.appcompat:appcompat:1.7.1")
}
