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

// The release signing key, read from a file that is deliberately not in the repo. Absent on a
// machine that has never been given the key, and a debug build still works there — only `release`
// needs it.
val keystoreFile = rootProject.file("keystore.properties")

val keystoreProperties = Properties().apply {
    if (keystoreFile.exists()) {
        keystoreFile.inputStream().use { load(it) }
    }
}

/**
 * Everything `signingConfigs` needs, or a list of what is missing.
 *
 * The four keys are checked together rather than one at a time, because a `keystore.properties`
 * that has three of them is a mistake worth naming all at once instead of one build at a time.
 */
val REQUIRED_KEYSTORE_KEYS = listOf("storeFile", "storePassword", "keyAlias", "keyPassword")

val missingKeystoreKeys = REQUIRED_KEYSTORE_KEYS.filter { keystoreProperties.getProperty(it).isNullOrBlank() }

val canSignRelease = keystoreFile.exists() && missingKeystoreKeys.isEmpty()

android {
    compileSdk = 36
    namespace = "com.shiver.mobile"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.shiver.mobile"
        minSdk = 24
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    signingConfigs {
        create("release") {
            // Populated only when there is a complete key to populate it with. An incomplete one is
            // caught by `failOnUnsignedRelease` below rather than half-applied here.
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
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            // Assigned only when it holds a key. An empty signing config is not a signing config,
            // and which of "fail loudly" or "quietly produce an unsigned apk" you get from one
            // depends on the AGP version — see `failOnUnsignedRelease`.
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
 * Refuses to build a release APK that would not be signed.
 *
 * The comment above `keystoreProperties` used to claim it was "better to fail at signing time than
 * to quietly ship an unsigned apk" — and then did not arrange for that. The signing config was
 * created empty when the key was absent and assigned to the release build type regardless, so
 * whether the build failed or produced an unsigned artifact came down to which AGP version was in
 * use. An unsigned release APK is not a broken build; it is a build that installs on the developer's
 * own phone and cannot be shipped, which is the kind of thing that is discovered at the worst
 * moment.
 *
 * **An unsigned APK is also unable to update an existing install**: Android identifies an app by
 * package name *and* signing certificate, so an APK signed with a different key — or none — is a
 * different app to the system. That is why this is worth a hard stop rather than a warning.
 *
 * Debug builds are untouched: they sign with the local debug key and are supposed to work on a
 * machine that has never seen the release key.
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