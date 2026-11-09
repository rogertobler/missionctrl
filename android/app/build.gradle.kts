import java.util.Properties
import org.gradle.api.tasks.Exec

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

// App languages: English in values/, every values-xx/strings.xml adds one. The list in the
// settings and Android's own app-language list come from these folders, nothing to keep by hand.
// Folder names: values-de, values-en-rGB, values-es-r419, values-b+sr+Latn (BCP 47) → de, en-GB, es-419, sr-Latn.
val appLocales: List<String> = listOf("en") + file("src/main/res").listFiles().orEmpty()
    .filter { it.isDirectory && File(it, "strings.xml").exists() }
    .mapNotNull { dir ->
        val q = dir.name.removePrefix("values-").takeIf { it != dir.name } ?: return@mapNotNull null
        when {
            q.startsWith("b+") -> q.removePrefix("b+").replace('+', '-')
            Regex("[a-z]{2,3}(-r([A-Z]{2}|[0-9]{3}))?").matches(q) -> q.replace("-r", "-")
            else -> null
        }
    }
    .sorted()

// Release signing: a properties file outside the repository (storeFile, storePassword, keyAlias),
// named in local.properties as `signing.properties=/path/to/signing.properties`. `-PsigningKey=…`
// picks another key from the same store (e.g. the upload key for Google Play). Without the file
// the release build stays unsigned.
val signing: Properties? = run {
    val local = rootProject.file("local.properties").takeIf { it.exists() } ?: return@run null
    val path = Properties().apply { local.inputStream().use { load(it) } }
        .getProperty("signing.properties") ?: return@run null
    val file = File(path)
    if (!file.exists()) {
        logger.warn("Release signing: $path not found, the release build stays unsigned.")
        return@run null
    }
    Properties().apply { file.inputStream().use { load(it) } }
}

android {
    namespace = "ch.inscribe.missionctrl"
    compileSdk = 37

    defaultConfig {
        applicationId = "ch.inscribe.missionctrl"
        minSdk = 29
        targetSdk = 37
        versionCode = 1
        versionName = "1.0.0"
        ndk { abiFilters += listOf("arm64-v8a") }
        buildConfigField("String", "APP_LOCALES", "\"${appLocales.joinToString(",")}\"")
    }

    signingConfigs {
        if (signing != null) create("release") {
            storeFile = File(signing.getProperty("storeFile"))
            storePassword = signing.getProperty("storePassword")
            keyAlias = (findProperty("signingKey") as String?) ?: signing.getProperty("keyAlias")
            keyPassword = signing.getProperty("storePassword")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("release")
        }
    }

    buildFeatures { compose = true; buildConfig = true }

    // Android 13+ lists the languages from values-xx/ under the app's settings (res/resources.properties).
    androidResources { generateLocaleConfig = true }

    // Every language in every install: the language is chosen in the app, after installing.
    bundle { language { enableSplit = false } }

    // Plain JVM tests of the text and look logic; Android calls answer with defaults.
    testOptions { unitTests.isReturnDefaultValues = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            jniLibs.directories.add(layout.buildDirectory.dir("rust/jniLibs").get().asFile.path)
            kotlin.directories.add(layout.buildDirectory.dir("rust/kotlin").get().asFile.path)
        }
    }
}

// --- Rust core -------------------------------------------------------------

val coreDir = rootProject.projectDir.resolve("../core")
val ndkDir = providers.environmentVariable("ANDROID_NDK_HOME")
    .orElse("${System.getProperty("user.home")}/Android/Sdk/ndk/30.0.16248370")

val buildRust by tasks.registering(Exec::class) {
    description = "Builds mc-ffi for Android with cargo-ndk"
    workingDir = coreDir
    environment("ANDROID_NDK_HOME", ndkDir.get())
    // whisper.cpp (mc-voice) builds with CMake's own NDK support and needs C++ at run time.
    environment("CMAKE_ANDROID_NDK", ndkDir.get())
    environment("CMAKE_ANDROID_API", "29")
    environment("CMAKE_ANDROID_ARCH_ABI", "arm64-v8a")
    environment("BINDGEN_EXTRA_CLANG_ARGS", "--target=aarch64-linux-android29")
    // Dot product and half floats: every arm64 phone since about 2019; several times faster.
    environment("GGML_CPU_ARM_ARCH", "armv8.2-a+dotprod+fp16")
    commandLine(
        "cargo", "ndk", "-t", "arm64-v8a", "-P", "29", "--link-libcxx-shared",
        "-o", layout.buildDirectory.dir("rust/jniLibs").get().asFile.path,
        "build", "-p", "mc-ffi", "--release",
    )
}

val generateBindings by tasks.registering(Exec::class) {
    description = "Generates the Kotlin bindings for mc-ffi"
    dependsOn(buildRust)
    workingDir = coreDir
    commandLine(
        "cargo", "run", "-q", "-p", "uniffi-bindgen", "--",
        "generate", "--no-format",
        "--library", "target/aarch64-linux-android/release/libmc_ffi.so",
        "--language", "kotlin",
        "--out-dir", layout.buildDirectory.dir("rust/kotlin").get().asFile.path,
    )
}

tasks.named("preBuild") { dependsOn(generateBindings) }

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(composeBom)
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
    // UniFFI's Kotlin bindings call into the Rust library through JNA.
    implementation("net.java.dev.jna:jna:5.19.1@aar")

    testImplementation("junit:junit:4.13.2")
    // org.json is part of Android; the JVM tests need the real thing.
    testImplementation("org.json:json:20240303")
}
