plugins {
    id("com.android.application")
}

// Single source of truth for the version: the workspace Cargo.toml.
// versionCode = major*10000 + minor*100 + patch (e.g. 0.2.0 -> 200).
val appVersion: String = Regex("""(?m)^\s*version\s*=\s*"([^"]+)"""")
    .find(rootProject.file("../Cargo.toml").readText())!!.groupValues[1]
val appVersionCode: Int = appVersion.split(".").map { it.toInt() }.let { (a, b, c) -> a * 10000 + b * 100 + c }

// Release signing comes from the environment (GitHub Actions secrets), never
// from files in the repository. Without it, release builds are unsigned.
val signingStoreFile: String? = System.getenv("ANDROID_KEYSTORE_FILE")
val signingPassword: String? = System.getenv("ANDROID_KEYSTORE_PASSWORD")
val signingAlias: String? = System.getenv("ANDROID_KEY_ALIAS")
val canSign = signingStoreFile != null && signingPassword != null && signingAlias != null

android {
    namespace = "io.github.vickdini.iptv"
    compileSdk = 35
    ndkVersion = "27.0.12077973"

    defaultConfig {
        applicationId = "io.github.vickdini.iptv"
        minSdk = 24
        targetSdk = 35
        versionCode = appVersionCode
        versionName = appVersion
    }

    signingConfigs {
        if (canSign) {
            create("release") {
                storeFile = file(signingStoreFile!!)
                storePassword = signingPassword
                keyAlias = signingAlias
                keyPassword = signingPassword
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (canSign) signingConfig = signingConfigs.getByName("release")
        }
    }

    // Kotlin targets the same JVM version (AGP 9 built-in Kotlin follows this).
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // The Rust core (crates/iptv-android) is built by cargo-ndk into here.
    sourceSets["main"].jniLibs.directories.add(layout.buildDirectory.dir("rustJniLibs").get().asFile.path)
}

// Builds the shared Rust core for every Android CPU type before the app.
// Requires: rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
//           cargo install cargo-ndk
val ndkDir = androidComponents.sdkComponents.ndkDirectory
val cargoBuild = tasks.register<Exec>("cargoBuild") {
    val repoRoot = rootProject.projectDir.parentFile
    val outDir = layout.buildDirectory.dir("rustJniLibs").get().asFile
    workingDir = repoRoot
    inputs.dir(File(repoRoot, "crates"))
    inputs.file(File(repoRoot, "Cargo.lock"))
    outputs.dir(outDir)
    doFirst { environment("ANDROID_NDK_HOME", ndkDir.get().asFile.absolutePath) }
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a", "-t", "armeabi-v7a", "-t", "x86_64",
        "-P", "24",
        "-o", outDir.absolutePath,
        "build", "--release", "-p", "iptv-android",
    )
}
tasks.named("preBuild") { dependsOn(cargoBuild) }

dependencies {
    val media3 = "1.5.0"
    implementation("androidx.media3:media3-exoplayer:$media3")
    implementation("androidx.media3:media3-exoplayer-hls:$media3")
    implementation("androidx.media3:media3-exoplayer-dash:$media3")
    implementation("androidx.media3:media3-ui:$media3")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.recyclerview:recyclerview:1.3.2")
}
