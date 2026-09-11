// The Archie capture agent for Android.
//
// A minimal, real Gradle build. It compiles to an APK the owner side-loads onto their own
// phone; it is not published to a store, because it is a personal tool for the owner's own
// devices, not an app for other people.
//
// Build:  ./gradlew assembleDebug
// Install: adb install build/outputs/apk/debug/*.apk
// Then grant Notification access in Settings and pair the phone to the node.

plugins {
    id("com.android.application") version "8.5.0"
    kotlin("android") version "2.0.0"
}

android {
    namespace = "os.archie.capture"
    compileSdk = 34

    defaultConfig {
        applicationId = "os.archie.capture"
        minSdk = 26 // API 26: NotificationListenerService extras and BiometricPrompt
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
    }

    sourceSets["main"].java.srcDirs("src/main/kotlin")

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.biometric:biometric:1.1.0")
    implementation("androidx.fragment:fragment-ktx:1.8.0")
}
