plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.ksp)
    alias(libs.plugins.hilt)
    alias(libs.plugins.ktlint)
}

// The short commit hash of the checkout, or "unknown" (no git, or not a
// checkout, as in a source tarball). Only hex digits get through.
val gitCommit: String =
    try {
        providers
            .exec {
                // The full hash, cut to 7 here: `--short` may print more digits in
                // a clone with more objects, which would break reproducibility.
                commandLine("git", "rev-parse", "HEAD")
                isIgnoreExitValue = true
            }.standardOutput.asText
            .get()
            .trim()
            .takeIf { it.length >= 40 && it.all { c -> c in '0'..'9' || c in 'a'..'f' } }
            ?.take(7)
            ?: "unknown"
    } catch (e: Exception) {
        "unknown"
    }

// The one place the application ID lives (it may change; see issue #79).
val mailcraneApplicationId = "app.mailcrane"

val coreDir = rootProject.layout.projectDirectory.dir("../core")
// The core for each build type has its own folder (rustJniLibs/debug and
// rustJniLibs/release), packaged only by that build type.
val rustJniLibsRoot = layout.buildDirectory.dir("rustJniLibs")
val uniffiDir = layout.buildDirectory.dir("generated/uniffi")
val rustRelease = gradle.startParameter.taskNames.any { it.contains("Release", ignoreCase = true) }

android {
    namespace = mailcraneApplicationId
    // Pinned so release builds are reproducible (docs/fdroid.md): the NDK's
    // linker and libc++ end up in the APK. cargoNdkBuild uses the same NDK.
    // To update, change it here and NDK_VERSION in .github/workflows/ci.yml.
    ndkVersion = "30.0.16248370"
    compileSdk {
        version = release(37) { minorApiLevel = 2 }
    }

    defaultConfig {
        applicationId = mailcraneApplicationId
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.0.0"
        // The commit this build comes from, shown in Settings → About. The same
        // commit gives the same value on any machine, so release builds stay
        // reproducible (docs/fdroid.md); without git it is "unknown".
        buildConfigField("String", "GIT_COMMIT", "\"$gitCommit\"")
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // Only the ABIs the Rust core is built for (cargoNdkBuild). Without this,
        // libraries such as JNA bring armeabi-v7a, armeabi and mips folders, and
        // Android would install the app on 32-bit phones, where it can't load the
        // core and crashes at once (found while preparing F-Droid, M6-04).
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        // Debug builds are a separate app, app.mailcrane.debug (#405), so they
        // install beside the release app on a real phone instead of replacing it
        // and its accounts. They also get their own OAuth redirect scheme, so they
        // can never catch the release app's sign-in; Google sign-in therefore
        // doesn't complete in debug builds.
        debug {
            applicationIdSuffix = ".debug"
            manifestPlaceholders["oauthScheme"] = "$mailcraneApplicationId.debug"
        }
        release {
            manifestPlaceholders["oauthScheme"] = mailcraneApplicationId
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
        // BuildConfig.GIT_COMMIT for Settings → About (#75).
        buildConfig = true
    }

    // Per-app language (Android 13+, M6-02): AGP lists every values-<locale>
    // folder in a generated locale config, so a language added through
    // Weblate shows up in the system settings without editing a file.
    // res/resources.properties names the language of values/ (English).
    androidResources {
        generateLocaleConfig = true
    }

    // A language half-done in Weblate falls back to English for the rest;
    // that mustn't fail the build. `cargo xtask ci` still checks that the
    // translated strings keep their placeholders.
    lint {
        warning += "MissingTranslation"
    }

    // Robolectric (Compose UI tests on the JVM) needs the app's resources.
    testOptions {
        unitTests.isIncludeAndroidResources = true
    }

    sourceSets.getByName("main") {
        kotlin.directories.add(uniffiDir.get().asFile.path)
    }
    // Each build type packages only the core built for it. With one shared
    // folder, a debug APK built after a release build shipped the release
    // core, which lacks the debug-only functions, and crashed at launch.
    for (buildType in listOf("debug", "release")) {
        sourceSets.getByName(buildType) {
            jniLibs.directories.add(
                rustJniLibsRoot
                    .get()
                    .dir(buildType)
                    .asFile.path,
            )
        }
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

// This run's core, and a marker naming the profile it was built with. The
// guards below read the marker before a variant packages the folder.
val rustJniLibsDir = rustJniLibsRoot.map { it.dir(rustProfile) }
val rustProfileMarker = rustJniLibsRoot.map { it.file("$rustProfile.profile") }

// Where Cargo puts its output: CARGO_TARGET_DIR if set (relative to core/, as
// Cargo treats it), otherwise core/target.
val cargoTargetDir: File =
    System.getenv("CARGO_TARGET_DIR")?.let { File(it) }?.let {
        if (it.isAbsolute) it else coreDir.file(it.path).asFile
    } ?: coreDir.dir("target").asFile

val ndkDir = androidComponents.sdkComponents.ndkDirectory

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
    outputs.file(rustProfileMarker)
    // Set here, not left to the shell: Gradle's daemon is shared and keeps the
    // environment it started with, so a worker's CARGO_* settings may not
    // reach this task. Incremental caches and full debug info filled the
    // disk with several builds running at once.
    environment("CARGO_INCREMENTAL", "0")
    environment("CARGO_PROFILE_DEV_DEBUG", "line-tables-only")
    // The pinned NDK (ndkVersion), not whichever one cargo-ndk would find first.
    doFirst { (this as Exec).environment("ANDROID_NDK_HOME", ndkDir.get().asFile.path) }
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
    val marker = rustProfileMarker
    val profile = rustProfile
    doLast { marker.get().asFile.writeText("$profile\n") }
}

// The core is built once per Gradle run, for debug or release according to
// the task names asked for (`rustRelease`), into that profile's folder. A
// variant may package its folder only when this run built it and the marker
// says so. Otherwise a run that packages a release variant without naming it
// (`assemble`, `build`) would leave the release folder stale or put a debug
// core, with the fixture loader (#289), in it; and a debug variant packaged in
// a release run would get a stale core that may not match its bindings.
fun Task.checkCoreProfile(
    buildType: String,
    hint: String,
) {
    val built = rustProfile
    val marker = rustJniLibsRoot.map { it.file("$buildType.profile") }
    group = "verification"
    description = "Fails when a $buildType variant would be packaged with a core not built for $buildType."
    dependsOn(cargoNdkBuild)
    doLast {
        if (built != buildType) {
            throw GradleException(
                "A $buildType variant needs a $buildType core, but this run builds the $built core: $hint",
            )
        }
        val found =
            marker
                .get()
                .asFile
                .takeIf { it.isFile }
                ?.readText()
                ?.trim()
        if (found != buildType) {
            throw GradleException(
                "rustJniLibs/$buildType holds a ${found ?: "unmarked"} core; delete app/build/rustJniLibs and build again.",
            )
        }
    }
}

val checkNoDebugCoreInRelease by tasks.registering {
    checkCoreProfile("release", "name the release task (assembleRelease, bundleRelease).")
}

val checkNoReleaseCoreInDebug by tasks.registering {
    checkCoreProfile("debug", "build debug and release variants in separate Gradle runs.")
}

tasks.matching { it.name == "mergeReleaseNativeLibs" || it.name == "mergeReleaseJniLibFolders" }.configureEach {
    dependsOn(checkNoDebugCoreInRelease)
}

tasks.matching { it.name == "mergeDebugNativeLibs" || it.name == "mergeDebugJniLibFolders" }.configureEach {
    dependsOn(checkNoReleaseCoreInDebug)
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

// Open-source licences (#366): Settings → About → Open-source licences reads
// assets/licences.json, built here from two sources, with no network at run time:
//   - the Rust core: licences/rust-licences.txt, written by `cargo xtask licences`
//     (cargo-about over mailcrane-ffi) and checked for freshness by `cargo xtask ci`;
//   - the Android libraries: each module of the release runtime classpath (the one
//     checkFreeDependencies reads) with the licence in its Maven POM.
// The build fails when a library's licence is missing, unknown or not on the
// allowed list in core/deny.toml, so nothing reaches a release unlisted.

// Libraries whose POM lists several licences as alternatives, and the one we use.
val dualLicensed =
    mapOf(
        "net.java.dev.jna:jna" to "Apache-2.0",
        "net.java.dev.jna:jna-platform" to "Apache-2.0",
    )

// Libraries whose POM names no licence, with the licence from their own source.
val licenceWithoutPom =
    mapOf(
        // Built from the rustls-platform-verifier-android crate: "MIT OR Apache-2.0".
        "org.rustls:rustls-platform-verifier" to "Apache-2.0",
    )

// The SPDX id for a POM <license>, or null when it isn't recognised.
fun spdxFor(
    name: String,
    url: String,
    allowed: List<String>,
): String? {
    val n = name.trim().lowercase()
    val u = url.trim().lowercase()
    allowed.firstOrNull { it.lowercase() == n }?.let { return it }
    return when {
        "apache.org/licenses/license-2.0" in u || ("apache" in n && "2" in n) -> "Apache-2.0"
        n == "mit" || "mit license" in n || "opensource.org/licenses/mit" in u -> "MIT"
        "bsd-3-clause" in n ||
            "bsd 3" in n ||
            "new bsd" in n ||
            "revised bsd" in n ||
            "opensource.org/licenses/bsd-3-clause" in u -> "BSD-3-Clause"
        "bsd-2-clause" in n || "bsd 2" in n || "simplified bsd" in n -> "BSD-2-Clause"
        "mozilla public license" in n && "2.0" in n -> "MPL-2.0"
        "lgpl" in n || "lesser general public" in n -> "LGPL"
        "gpl" in n || "general public license" in n -> "GPL"
        else -> null
    }
}

// <licenses> of the POM for group:name:version, following <parent> when the POM
// has none (up to 5 levels). Uses the repositories the build already uses.
fun pomLicences(
    group: String,
    name: String,
    version: String,
    depth: Int = 0,
): List<Pair<String, String>> {
    if (depth > 5) return emptyList()
    val result =
        dependencies
            .createArtifactResolutionQuery()
            .forModule(group, name, version)
            .withArtifacts(MavenModule::class.java, MavenPomArtifact::class.java)
            .execute()
    val pom =
        result.resolvedComponents
            .flatMap { it.getArtifacts(MavenPomArtifact::class.java) }
            .filterIsInstance<org.gradle.api.artifacts.result.ResolvedArtifactResult>()
            .firstOrNull()
            ?.file ?: return emptyList()
    val factory =
        javax.xml.parsers.DocumentBuilderFactory
            .newInstance()
    // POMs are data: no DOCTYPEs, no external entities.
    factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
    factory.isExpandEntityReferences = false
    val root = factory.newDocumentBuilder().parse(pom).documentElement

    fun children(
        parent: org.w3c.dom.Element,
        tag: String,
    ): List<org.w3c.dom.Element> =
        (0 until parent.childNodes.length)
            .map { parent.childNodes.item(it) }
            .filterIsInstance<org.w3c.dom.Element>()
            .filter { it.tagName == tag }

    fun text(
        parent: org.w3c.dom.Element,
        tag: String,
    ) = children(parent, tag)
        .firstOrNull()
        ?.textContent
        ?.trim()
        .orEmpty()

    val licences =
        children(root, "licenses").flatMap { children(it, "license") }.map { text(it, "name") to text(it, "url") }
    if (licences.isNotEmpty()) return licences
    val parent = children(root, "parent").firstOrNull() ?: return emptyList()
    return pomLicences(text(parent, "groupId"), text(parent, "artifactId"), text(parent, "version"), depth + 1)
}

// The `allow` list in core/deny.toml, which cargo deny enforces for Rust.
val allowedLicences: List<String> =
    coreDir
        .file("deny.toml")
        .asFile
        .readLines()
        .dropWhile { !it.trim().startsWith("allow = [") }
        .drop(1)
        .takeWhile { !it.trim().startsWith("]") }
        .mapNotNull { Regex("^\\s*\"([^\"]+)\"").find(it)?.groupValues?.get(1) }

// "group:name:version<TAB>SPDX id", or "<TAB>!problem" for a licence that can't be used.
// Lazy: AGP creates the configuration after this script runs.
val androidLibraryLicences: Provider<List<String>> =
    providers.provider {
        configurations
            .getByName("releaseRuntimeClasspath")
            .incoming.resolutionResult.allComponents
            .mapNotNull { it.id as? org.gradle.api.artifacts.component.ModuleComponentIdentifier }
            .sortedBy { "${it.group}:${it.module}" }
            .map { id ->
                val key = "${id.group}:${id.module}"
                val found = pomLicences(id.group, id.module, id.version)
                val ids = found.map { (n, u) -> spdxFor(n, u, allowedLicences) }
                val dual = dualLicensed[key]
                val chosen =
                    when {
                        found.isEmpty() && key in licenceWithoutPom -> licenceWithoutPom.getValue(key)
                        found.isEmpty() -> "!no licence in its POM"
                        dual != null && dual in ids -> dual
                        ids.any { it == null } -> "!unrecognised licence ${found.map { it.first }}"
                        ids.all { it in allowedLicences } -> ids.filterNotNull().first()
                        else -> "!not allowed: ${ids.filterNot { it in allowedLicences }} (AGENTS.md §5)"
                    }
                "${id.group}:${id.module}:${id.version}\t$chosen"
            }
    }

@CacheableTask
abstract class GenerateLicences : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val rustNotices: RegularFileProperty

    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val texts: DirectoryProperty

    @get:Input
    abstract val androidLibraries: ListProperty<String>

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @TaskAction
    fun generate() {
        val textMap = linkedMapOf<String, Map<String, String>>()
        val libraries = mutableListOf<Map<String, String>>()
        readRust(textMap, libraries)
        if (libraries.isEmpty()) {
            throw GradleException("licences/rust-licences.txt lists no crates; run `cargo xtask licences`")
        }
        val problems = mutableListOf<String>()
        for (entry in androidLibraries.get()) {
            val (coordinates, licence) = entry.split('\t', limit = 2)
            if (licence.startsWith("!")) {
                problems += "$coordinates: ${licence.drop(1)}"
                continue
            }
            val textFile = texts.get().file("$licence.txt").asFile
            if (!textFile.isFile) {
                problems += "$coordinates: $licence has no text; add android/app/licences/texts/$licence.txt"
                continue
            }
            val key = "spdx-$licence"
            textMap.getOrPut(key) { mapOf("id" to licence, "text" to textFile.readText().trimEnd()) }
            val parts = coordinates.split(':')
            libraries +=
                mapOf(
                    "name" to "${parts[0]}:${parts.getOrElse(1) { "" }}",
                    "version" to parts.getOrElse(2) { "" },
                    "kind" to "android",
                    "licence" to licence,
                    "text" to key,
                    "url" to "",
                )
        }
        if (problems.isNotEmpty()) {
            throw GradleException(
                "Open-source licences (#366): these release libraries can't be listed:\n" +
                    problems.joinToString("\n") { "  $it" },
            )
        }
        libraries.sortWith(compareBy({ it.getValue("name").lowercase() }, { it.getValue("version") }))
        val json = groovy.json.JsonOutput.toJson(mapOf("texts" to textMap, "libraries" to libraries))
        val out = outputDir.get().asFile
        out.deleteRecursively()
        out.mkdirs()
        out.resolve("licences.json").writeText(json + "\n")
        logger.lifecycle("Open-source licences: ${libraries.size} libraries, ${textMap.size} licence texts.")
    }

    // Blocks of "@@licence <id>", "@@crate <name> <version> [<repository>]"…, "@@text",
    // the licence text, then "@@end" (written by licences/rust-licences.hbs).
    private fun readRust(
        textMap: MutableMap<String, Map<String, String>>,
        libraries: MutableList<Map<String, String>>,
    ) {
        var id = ""
        var crates = mutableListOf<List<String>>()
        var inText = false
        val body = StringBuilder()
        rustNotices.get().asFile.readLines().forEach { line ->
            when {
                inText && line == "@@end" -> {
                    val key = "rust-${textMap.size}"
                    textMap[key] = mapOf("id" to id, "text" to body.toString().trimEnd())
                    crates.forEach { c ->
                        libraries +=
                            mapOf(
                                "name" to c[0],
                                "version" to c.getOrElse(1) { "" },
                                "kind" to "rust",
                                "licence" to id,
                                "text" to key,
                                "url" to c.getOrElse(2) { "" },
                            )
                    }
                    inText = false
                }
                inText -> body.append(line).append('\n')
                line.startsWith("@@licence ") -> {
                    id = line.removePrefix("@@licence ").trim()
                    crates = mutableListOf()
                    body.clear()
                }
                line.startsWith("@@crate ") -> crates += line.removePrefix("@@crate ").trim().split(' ')
                line == "@@text" -> inText = true
            }
        }
    }
}

val generateLicences by tasks.registering(GenerateLicences::class) {
    rustNotices.set(layout.projectDirectory.file("licences/rust-licences.txt"))
    texts.set(layout.projectDirectory.dir("licences/texts"))
    androidLibraries.set(androidLibraryLicences)
}

androidComponents {
    onVariants { variant ->
        variant.sources.assets?.addGeneratedSourceDirectory(generateLicences, GenerateLicences::outputDir)
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
    // Optional push wake-ups through a UnifiedPush distributor (M4-05). Free; no FCM.
    implementation(libs.unifiedpush.connector)
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
