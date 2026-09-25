import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// The release signing key, from a file kept out of the repo (see RELEASING.md). Debug builds do not need it.
val keystoreFile = rootProject.file("keystore.properties")

val keystoreProperties = Properties().apply {
    if (keystoreFile.exists()) {
        keystoreFile.inputStream().use { load(it) }
    }
}

val REQUIRED_KEYSTORE_KEYS = listOf("storeFile", "storePassword", "keyAlias", "keyPassword")

val missingKeystoreKeys = REQUIRED_KEYSTORE_KEYS.filter { keystoreProperties.getProperty(it).isNullOrBlank() }

val canSignRelease = keystoreFile.exists() && missingKeystoreKeys.isEmpty()

android {
    compileSdk = 36
    namespace = "com.shiver.mobile"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.shiver.mobile"
        minSdk = 26
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    signingConfigs {
        create("release") {
            if (canSignRelease) {
                storeFile = file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            signingConfig = if (canSignRelease) signingConfigs.getByName("release") else null
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

/**
 * Refuses to build an unsigned release (whether AGP fails or silently produces one varies by
 * version, and an APK not signed with Shiver's key cannot update an existing install).
 */
val failOnUnsignedRelease = tasks.register("failOnUnsignedRelease") {
    doFirst {
        if (canSignRelease) return@doFirst

        val reason = if (!keystoreFile.exists()) {
            "${keystoreFile.absolutePath} does not exist"
        } else {
            "${keystoreFile.name} is missing: ${missingKeystoreKeys.joinToString(", ")}"
        }

        throw GradleException(
            """
            Refusing to build an unsigned release APK.

            $reason

            A release APK has to be signed with Shiver's release key, and it has to be *that* key:
            Android identifies an app by its package name and its signing certificate together, so
            an APK signed with anything else cannot update an existing install.

            See RELEASING.md for where the key lives and how to restore it. To build something
            runnable without it, use a debug build:

                ./node_modules/.bin/tauri android build --apk --debug
            """.trimIndent()
        )
    }
}

// every release-variant assembly waits on the check, so there is no route to an unsigned artifact
tasks.matching { it.name.startsWith("assembleRelease") || it.name.startsWith("bundleRelease") }
    .configureEach { dependsOn(failOnUnsignedRelease) }

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")