plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.shiver.rail"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }

    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    // the back press that closes the rail (the app's own version)
    implementation("androidx.activity:activity-ktx:1.10.1")
    // the rail's WebView: its page from the assets, and the message channel to it (the app's own version)
    implementation("androidx.webkit:webkit:1.14.0")
    // the activity type Tauri's plugin lifecycle hands over (the version tauri-android uses)
    implementation("androidx.appcompat:appcompat:1.6.0")
    implementation(project(":tauri-android"))
}
