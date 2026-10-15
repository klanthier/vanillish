// The APK: Java glue (VanillishActivity, Bridge, GamepadBridge, …) around libvanillish.so, which
// `android/build-rust.sh` builds with cargo-ndk into src/main/jniLibs before Gradle runs.
import java.util.Properties

plugins {
    id("com.android.application")
}

// The version pins (SDK levels, NDK, GameActivity) live in android/gradle.properties, where
// build-rust.sh and the workflows read the same lines.
fun pin(name: String): String = providers.gradleProperty("vanillish.$name").get()

// Release signing comes from the environment (CI decodes its secret to a file and points
// VANILLISH_KEYSTORE at it) or `android/keystore.properties`; without either, a release build is
// signed with this machine's debug key so it still installs. One key, forever: Android refuses an
// update signed with another one, and uninstalling to get past that deletes the app's data — the
// game data, the settings, the SavedVariables (`Android/data/<package>/files`).
val signing = Properties().apply {
    val f = rootProject.file("keystore.properties")
    if (f.exists()) f.inputStream().use { load(it) }
    System.getenv("VANILLISH_KEYSTORE")?.takeIf { it.isNotEmpty() }?.let { setProperty("storeFile", it) }
    System.getenv("VANILLISH_KEYSTORE_PASSWORD")?.takeIf { it.isNotEmpty() }?.let { setProperty("storePassword", it) }
    System.getenv("VANILLISH_KEY_ALIAS")?.takeIf { it.isNotEmpty() }?.let { setProperty("keyAlias", it) }
    System.getenv("VANILLISH_KEY_PASSWORD")?.takeIf { it.isNotEmpty() }?.let { setProperty("keyPassword", it) }
}
val releaseKeystore: File? = signing.getProperty("storeFile")?.let { file(it) }
if (releaseKeystore != null) {
    // Said here, by name, rather than as AGP's "keystore password was incorrect" at the very end.
    check(releaseKeystore.isFile) { "vanillish: the release keystore $releaseKeystore does not exist" }
    val missing = listOf("storePassword", "keyAlias").filter { signing.getProperty(it).isNullOrEmpty() }
    check(missing.isEmpty()) {
        "vanillish: a release keystore is set but not ${missing.joinToString(" and ")} " +
            "(VANILLISH_KEYSTORE_PASSWORD / VANILLISH_KEY_ALIAS, or keystore.properties)"
    }
}

// The version. build-apk.sh passes a release's own (-Pvanillish.versionName=0.3.0 from the tag);
// otherwise it is the workspace version plus the commit, and the versionCode is the commit's time in
// minutes: it only grows along history, and a local build and a CI build of one commit agree, so
// any newer build installs over any older one without the uninstall that would delete the data.
val workspaceVersion: String = rootProject.file("../Cargo.toml").takeIf { it.isFile }?.readLines()
    ?.dropWhile { it.trim() != "[workspace.package]" }
    ?.firstNotNullOfOrNull { Regex("""^version\s*=\s*"([^"]+)"""").find(it.trim())?.groupValues?.get(1) }
    ?: "0.0.0"
fun git(vararg args: String): String? = runCatching {
    providers.exec {
        commandLine("git", "-C", rootDir.path, *args)
        isIgnoreExitValue = true
    }.standardOutput.asText.get().trim().takeIf { it.isNotEmpty() }
}.getOrNull()
val versionNameProp: String = providers.gradleProperty("vanillish.versionName").orNull
    ?: git("rev-parse", "--short", "HEAD")?.let { "$workspaceVersion+$it" }
    ?: "$workspaceVersion+unknown"
val versionCodeProp: Int = providers.gradleProperty("vanillish.versionCode").orNull?.toInt()
    ?: git("log", "-1", "--format=%ct", "HEAD")?.toLongOrNull()?.let { (it / 60).toInt() }
    ?: 1

android {
    namespace = "io.github.klanthier.vanillish"
    compileSdk = pin("compileSdk").toInt()
    buildToolsVersion = pin("buildTools")
    // AGP's strip and symbol tasks use this NDK; cargo-ndk builds the library with the same one.
    ndkVersion = pin("ndkVersion")

    defaultConfig {
        applicationId = "io.github.klanthier.vanillish"
        minSdk = pin("minSdk").toInt()
        targetSdk = pin("targetSdk").toInt()
        versionCode = versionCodeProp
        versionName = versionNameProp
        ndk {
            // The Galaxy Z Fold line (and every current Android phone) is arm64.
            abiFilters += listOf("arm64-v8a")
        }
    }

    signingConfigs {
        if (releaseKeystore != null) {
            create("release") {
                storeFile = releaseKeystore
                storePassword = signing.getProperty("storePassword")
                keyAlias = signing.getProperty("keyAlias")
                // A PKCS12 keystore (keytool's default) has one password for the store and the key.
                keyPassword = signing.getProperty("keyPassword") ?: signing.getProperty("storePassword")
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
            // With the release key when there is one, so one CI debug build updates the last
            // (each CI runner would otherwise make up a debug key of its own).
            signingConfigs.findByName("release")?.let { signingConfig = it }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        jniLibs {
            // Keep the symbol tables: a panic's backtrace in logcat then names functions. The
            // APK's zip compression takes most of the size back. (build-rust.sh drops only the
            // DWARF, and keeps a copy with it in target/vanillish-out/symbols/.)
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

// The Rust half is built outside Gradle: say so, rather than package an APK that dies at launch
// with "dlopen failed: library libvanillish.so not found". Only the packaging path checks, so
// compiling the Java and lint (vanillish-check.yml) need no library.
val rustLib = layout.projectDirectory.file("src/main/jniLibs/arm64-v8a/libvanillish.so").asFile
tasks.matching { it.name.startsWith("merge") && it.name.endsWith("JniLibFolders") }.configureEach {
    doFirst {
        check(rustLib.isFile) { "vanillish: no $rustLib — run android/build-rust.sh (or android/build-apk.sh) first" }
    }
}

dependencies {
    // GameActivity: the Activity and the C glue android-activity vendors — the exact release
    // pinned beside it in gradle.properties (no prefab: the C side is compiled into
    // libvanillish.so by android-activity's build.rs).
    implementation("androidx.games:games-activity:${pin("gamesActivity")}")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.core:core:1.17.0")
    // The fold: FoldingFeature (tabletop / book postures), through the callback API for Java.
    implementation("androidx.window:window:1.5.1")
    implementation("androidx.window:window-java:1.5.1")
}
