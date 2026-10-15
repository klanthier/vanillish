// The APK: Java glue (VanillishActivity, Bridge, GamepadBridge, …) around libvanillish.so, which
// `android/build-rust.sh` builds with cargo-ndk into src/main/jniLibs before Gradle runs.
import java.util.Properties

plugins {
    id("com.android.application")
}

// Release signing comes from the environment (CI secrets) or `android/keystore.properties`; without
// either, a release build is signed with the debug key so it still installs.
val signing = Properties().apply {
    val f = rootProject.file("keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
    System.getenv("VANILLISH_KEYSTORE")?.let { setProperty("storeFile", it) }
    System.getenv("VANILLISH_KEYSTORE_PASSWORD")?.let { setProperty("storePassword", it) }
    System.getenv("VANILLISH_KEY_ALIAS")?.let { setProperty("keyAlias", it) }
    System.getenv("VANILLISH_KEY_PASSWORD")?.let { setProperty("keyPassword", it) }
}

val versionNameProp = (findProperty("vanillish.versionName") as String?) ?: "0.2.0"
val versionCodeProp = ((findProperty("vanillish.versionCode") as String?) ?: "1").toInt()

android {
    namespace = "io.github.klanthier.vanillish"
    compileSdk = 36
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "io.github.klanthier.vanillish"
        // Android 12: VibratorManager, AThermal headroom, the cutout mode the game draws under.
        minSdk = 31
        targetSdk = 36
        versionCode = versionCodeProp
        versionName = versionNameProp
        ndk {
            // The Galaxy Z Fold line (and every current Android phone) is arm64.
            abiFilters += listOf("arm64-v8a")
        }
    }

    signingConfigs {
        if (signing.getProperty("storeFile") != null) {
            create("release") {
                storeFile = file(signing.getProperty("storeFile"))
                storePassword = signing.getProperty("storePassword")
                keyAlias = signing.getProperty("keyAlias")
                keyPassword = signing.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
        debug {
            // The Rust library is the same release-optimized build either way; this only makes
            // the Java side debuggable (and lets intent extras set WOW_*/INILLA_* knobs).
            applicationIdSuffix = ".debug"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        jniLibs {
            // Keep the symbol tables: a panic's backtrace in logcat then names functions. The
            // APK's zip compression takes most of the size back.
            keepDebugSymbols += "**/libvanillish.so"
            // Extracted to disk at install, as the loader is fastest with it.
            useLegacyPackaging = true
        }
    }

    lint {
        // jniLibs arrive from cargo-ndk; nothing else for lint to gate a release on.
        abortOnError = false
    }
}

dependencies {
    // GameActivity: the Activity and the C glue android-activity 0.6.1 vendors (4.4.0 exactly;
    // no prefab — the C side is compiled into libvanillish.so by android-activity's build.rs).
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.core:core:1.17.0")
    // The fold: FoldingFeature (tabletop / book postures), through the callback API for Java.
    implementation("androidx.window:window:1.5.1")
    implementation("androidx.window:window-java:1.5.1")
}
