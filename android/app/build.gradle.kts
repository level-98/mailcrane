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

    sourceSets.getByName("main") {
        jniLibs.directories.add(rustJniLibsDir.get().asFile.path)
        kotlin.directories.add(uniffiDir.get().asFile.path)
    }
}

kotlin {
    jvmToolchain(17)
}

// Builds the Rust core for Android, then generates the Kotlin bindings from the
// built library. Nothing it produces is committed.
val buildRust by tasks.registering(Exec::class) {
    group = "build"
    description = "Builds mailcrane-ffi with cargo-ndk and generates the UniFFI Kotlin bindings."
    workingDir = coreDir.asFile
    val outJni = rustJniLibsDir.get().asFile
    val outUniffi = uniffiDir.get().asFile
    val profile = if (rustRelease) "release" else "debug"
    inputs.dir(coreDir.dir("crates"))
    inputs.files(coreDir.file("Cargo.toml"), coreDir.file("Cargo.lock"))
    outputs.dir(outJni)
    outputs.dir(outUniffi)
    commandLine(
        "sh",
        "-c",
        """
        set -eu
        cargo ndk -t arm64-v8a -t x86_64 -o "${'$'}1" build -p mailcrane-ffi ${if (rustRelease) "--release" else ""}
        cargo run -q -p mailcrane-ffi --features bindgen --bin uniffi-bindgen -- \
            generate --library "${'$'}{CARGO_TARGET_DIR:-target}/aarch64-linux-android/$profile/libmailcrane_ffi.so" \
            --language kotlin --out-dir "${'$'}2"
        """.trimIndent(),
        "buildRust",
        outJni.path,
        outUniffi.path,
    )
}

tasks.named("preBuild") {
    dependsOn(buildRust)
}

// ktlint scans the source sets, which include the generated bindings.
tasks.matching { it.name.startsWith("runKtlint") }.configureEach {
    mustRunAfter(buildRust)
}

// ktlint must not look at generated bindings.
ktlint {
    filter {
        exclude { it.file.path.contains("/build/") }
    }
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.hilt.android)
    implementation(libs.hilt.navigation.compose)
    ksp(libs.hilt.compiler)
    implementation("${libs.jna.get()}@aar")

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
}
