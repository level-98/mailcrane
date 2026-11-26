plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.ksp)
    alias(libs.plugins.hilt)
    alias(libs.plugins.ktlint)
}

// The one place the application ID lives (it may change; see issue #79).
val mailcraneApplicationId = "app.mailcrane"

val coreDir = rootProject.layout.projectDirectory.dir("../core")
val rustJniLibsDir = layout.buildDirectory.dir("rustJniLibs")
val uniffiDir = layout.buildDirectory.dir("generated/uniffi")
val rustRelease = gradle.startParameter.taskNames.any { it.contains("Release", ignoreCase = true) }

android {
    namespace = mailcraneApplicationId
    compileSdk {
        version = release(37) { minorApiLevel = 2 }
    }

    defaultConfig {
        applicationId = mailcraneApplicationId
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.0.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // Only the ABIs the Rust core is built for (cargoNdkBuild). Without this,
        // libraries such as JNA bring armeabi-v7a, armeabi and mips folders, and
        // Android would install the app on 32-bit phones, where it can't load the
        // core and crashes at once (found while preparing F-Droid, M6-04).
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // No git commit in the APK, so builds from any checkout match (F-Droid
            // reproducible builds, M6-04).
            vcsInfo { include = false }
        }
    }

    // AGP otherwise adds a dependency-metadata block encrypted with a Google key;
    // F-Droid flags it as a non-free blob (M6-04).
    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    // Robolectric (Compose UI tests on the JVM) needs the app's resources.
    testOptions {
        unitTests.isIncludeAndroidResources = true
    }

    sourceSets.getByName("main") {
        jniLibs.directories.add(rustJniLibsDir.get().asFile.path)
        kotlin.directories.add(uniffiDir.get().asFile.path)
    }
}

// Compile for Java 17 with whichever JDK (17 or newer) runs Gradle, instead of
// requiring an exact JDK 17 toolchain that most machines don't have.
kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

// Builds the Rust core for Android, then generates the Kotlin bindings from the
// built library. Nothing it produces is committed. Cargo runs directly, with no
// shell in between, so this works the same on Windows, macOS and Linux.
val rustProfile = if (rustRelease) "release" else "debug"

// "Load test mail" (#289) exists only in debug builds: the core gets the
// fixture-loader feature for debug, never for release. `cargo xtask ci`
// checks that this stays the only line that turns it on.
val rustFeatures = if (rustRelease) emptyList() else listOf("--features", "debug-fixtures")

// Where Cargo puts its output: CARGO_TARGET_DIR if set (relative to core/, as
// Cargo treats it), otherwise core/target.
val cargoTargetDir: File =
    System.getenv("CARGO_TARGET_DIR")?.let { File(it) }?.let {
        if (it.isAbsolute) it else coreDir.file(it.path).asFile
    } ?: coreDir.dir("target").asFile

val cargoNdkBuild by tasks.registering(Exec::class) {
    group = "build"
    description = "Builds mailcrane-ffi for arm64-v8a and x86_64 with cargo-ndk."
    workingDir = coreDir.asFile
    inputs.dir(coreDir.dir("crates"))
    inputs.files(coreDir.file("Cargo.toml"), coreDir.file("Cargo.lock"))
    // Debug and release builds share the output folder; a change of profile or
    // features must rebuild the library.
    inputs.property("rustProfile", rustProfile)
    inputs.property("rustFeatures", rustFeatures)
    outputs.dir(rustJniLibsDir)
    // Set here, not left to the shell: Gradle's daemon is shared and keeps the
    // environment it started with, so a worker's CARGO_* settings may not
    // reach this task. Incremental caches and full debug info filled the
    // disk with several builds running at once.
    environment("CARGO_INCREMENTAL", "0")
    environment("CARGO_PROFILE_DEV_DEBUG", "line-tables-only")
    // Release builds don't embed this machine's paths (the checkout and the
    // cargo registry), so the same source builds the same library anywhere
    // (F-Droid reproducible builds, M6-04; docs/fdroid.md). Debug builds are
    // left alone: a changed RUSTFLAGS would rebuild every shared build folder.
    if (rustRelease) {
        val cargoHome = System.getenv("CARGO_HOME") ?: "${System.getProperty("user.home")}/.cargo"
        val remap =
            listOf(
                "--remap-path-prefix=${coreDir.asFile.path}=/mailcrane/core",
                "--remap-path-prefix=$cargoHome=/cargo",
            )
        environment("RUSTFLAGS", (listOfNotNull(System.getenv("RUSTFLAGS")) + remap).joinToString(" "))
    }
    commandLine(
        listOf(
            "cargo",
            "ndk",
            "-t",
            "arm64-v8a",
            "-t",
            "x86_64",
            "-o",
            rustJniLibsDir.get().asFile.path,
            "build",
            "-p",
            "mailcrane-ffi",
        ) + rustFeatures + if (rustRelease) listOf("--release") else emptyList(),
    )
}

// The core is built once per Gradle run, for debug or release according to
// the task names asked for (`rustRelease`). A run that would package a release
// variant without naming it (`assemble`, `build`) would put the debug core,
// with the fixture loader (#289), into the release APK: refuse it.
val checkNoDebugCoreInRelease by tasks.registering {
    group = "verification"
    description = "Fails when a release variant would be packaged with a core built for debug."
    val release = rustRelease
    doLast {
        if (!release) {
            throw GradleException(
                "A release variant needs a release core: name the release task (assembleRelease, bundleRelease).",
            )
        }
    }
}

tasks.matching { it.name == "mergeReleaseNativeLibs" || it.name == "mergeReleaseJniLibFolders" }.configureEach {
    dependsOn(checkNoDebugCoreInRelease)
}

// F-Droid (M6-04, #71): the release build must contain only free software and
// talk to no proprietary service. These groups are Google Play Services,
// Firebase, ML Kit, Play Core and billing, and common closed analytics or
// crash SDKs. The check reads the resolved release classpath, so a banned
// library pulled in by another one is caught too.
val nonFreeGroups =
    listOf(
        "com.google.android.gms",
        "com.google.firebase",
        "com.google.mlkit",
        "com.google.android.play",
        "com.android.billingclient",
        "com.google.android.datatransport",
        "com.google.android.libraries.places",
        "com.google.ads",
        "com.crashlytics",
        "io.fabric",
        "com.huawei.hms",
        "com.facebook",
        "com.appsflyer",
        "io.sentry",
        "com.amplitude",
        "com.mixpanel",
    )

val checkFreeDependencies by tasks.registering {
    group = "verification"
    description =
        "Fails when the release build depends on non-free libraries or ships prebuilt binaries in the sources."
    val classpath = configurations.named("releaseRuntimeClasspath")
    val sources = layout.projectDirectory.dir("src")
    val banned = nonFreeGroups
    doLast {
        val modules =
            classpath
                .get()
                .incoming.resolutionResult.allComponents
                .mapNotNull { it.moduleVersion }
        val found =
            modules.filter { m -> banned.any { m.group == it || m.group.startsWith("$it.") } }
        // Binaries in the sources would be blobs F-Droid can't build. Native
        // code comes only from the Rust core (built by cargoNdkBuild into
        // build/), and Java libraries only from Maven.
        val blobs =
            sources.asFileTree
                .matching { include("**/*.jar", "**/*.aar", "**/*.so", "**/*.dex", "**/*.apk") }
                .files
        val problems =
            found.map { "non-free dependency: ${it.group}:${it.name}:${it.version}" } +
                blobs.map { "prebuilt binary in the sources: ${it.relativeTo(sources.asFile)}" }
        if (problems.isNotEmpty()) {
            throw GradleException("F-Droid check failed:\n" + problems.joinToString("\n") { "  $it" })
        }
        logger.lifecycle("F-Droid check: ${modules.size} release dependencies, all free; no prebuilt binaries.")
    }
}

tasks.named("check") { dependsOn(checkFreeDependencies) }

// Debug builds carry tests/fixtures (`.eml` and `.html`) as assets under
// `fixtures/`, for "Load test mail" (#289). Release builds don't.
abstract class CopyFixtures : DefaultTask() {
    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val fixtures: DirectoryProperty

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @TaskAction
    fun copy() {
        val source = fixtures.get().asFile
        val target = outputDir.get().asFile.resolve("fixtures")
        target.deleteRecursively()
        source
            .walkTopDown()
            .filter { it.isFile && it.extension.lowercase() in setOf("eml", "html") }
            .forEach { it.copyTo(target.resolve(it.relativeTo(source)), overwrite = true) }
    }
}

val copyFixtureAssets by tasks.registering(CopyFixtures::class) {
    fixtures.set(rootProject.layout.projectDirectory.dir("../tests/fixtures"))
}

androidComponents {
    onVariants(selector().withBuildType("debug")) { variant ->
        variant.sources.assets?.addGeneratedSourceDirectory(copyFixtureAssets, CopyFixtures::outputDir)
    }
}

val buildRust by tasks.registering(Exec::class) {
    group = "build"
    description = "Builds the Rust core for Android and generates the UniFFI Kotlin bindings."
    dependsOn(cargoNdkBuild)
    workingDir = coreDir.asFile
    inputs.dir(rustJniLibsDir)
    // Binding options (package name, Android mode) change the generated code.
    inputs.file(coreDir.file("crates/mailcrane-ffi/uniffi.toml"))
    outputs.dir(uniffiDir)
    // Set here, not left to the shell: Gradle's daemon is shared and keeps the
    // environment it started with, so a worker's CARGO_* settings may not
    // reach this task. Incremental caches and full debug info filled the
    // disk with several builds running at once.
    environment("CARGO_INCREMENTAL", "0")
    environment("CARGO_PROFILE_DEV_DEBUG", "line-tables-only")
    commandLine(
        "cargo",
        "run",
        "-q",
        "-p",
        "mailcrane-ffi",
        "--features",
        "bindgen",
        "--bin",
        "uniffi-bindgen",
        "--",
        "generate",
        "--library",
        cargoTargetDir.resolve("aarch64-linux-android/$rustProfile/libmailcrane_ffi.so").path,
        "--language",
        "kotlin",
        "--out-dir",
        uniffiDir.get().asFile.path,
    )
}

// The Kotlin component of rustls-platform-verifier must be the version the Rust crate expects,
// or certificate checks fail at run time. Cargo.lock is the source of truth.
val checkRustlsVerifierVersion by tasks.registering {
    group = "verification"
    description = "Checks the rustls-platform-verifier Android component matches Cargo.lock."
    val lockFile = coreDir.file("Cargo.lock").asFile
    val expected = libs.versions.rustlsPlatformVerifier.get()
    inputs.file(lockFile)
    doLast {
        val lines = lockFile.readLines()
        val at = lines.indexOfFirst { it.trim() == "name = \"rustls-platform-verifier-android\"" }
        val locked =
            lines
                .drop(at + 1)
                .firstOrNull { it.trim().startsWith("version = ") }
                ?.substringAfter('"')
                ?.substringBefore('"')
        if (at < 0 || locked != expected) {
            throw GradleException(
                "rustlsPlatformVerifier in libs.versions.toml is $expected but core/Cargo.lock has $locked",
            )
        }
    }
}

tasks.named("preBuild") {
    dependsOn(buildRust, checkRustlsVerifierVersion)
}

// ktlint scans the source sets, which include the generated bindings.
tasks.matching { it.name.startsWith("runKtlint") }.configureEach {
    mustRunAfter(buildRust)
}

// ktlint must not look at generated bindings. Compare with "/" on every OS,
// since Windows paths use backslashes.
ktlint {
    filter {
        exclude { it.file.invariantSeparatorsPath.contains("/build/") }
    }
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.rustls.platform.verifier)
    implementation(libs.androidx.biometric)
    implementation(libs.androidx.browser)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.tooling.preview)
    debugImplementation(libs.androidx.compose.ui.tooling)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.hilt.android)
    implementation(libs.hilt.navigation.compose)
    ksp(libs.hilt.compiler)
    implementation("${libs.jna.get()}@aar")
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.androidx.work.runtime.ktx)
    implementation(libs.androidx.glance.appwidget)
    implementation(libs.androidx.glance.material3)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(platform(libs.androidx.compose.bom))
    testImplementation(libs.androidx.compose.ui.test.junit4)
    testImplementation(libs.robolectric)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.runner)
}
