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
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
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
    outputs.dir(rustJniLibsDir)
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
        ) + if (rustRelease) listOf("--release") else emptyList(),
    )
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
