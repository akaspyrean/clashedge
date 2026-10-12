import java.util.Base64

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.serialization)
}

// 版本号控制与 Windows 一致（src-tauri/src/update/mod.rs::current_version）：
// release tag 是唯一版本来源，CI 通过 CLASHEDGE_RELEASE_VERSION 注入；
// 本地开发构建回退为 "dev"。
val releaseVersion: String = System.getenv("CLASHEDGE_RELEASE_VERSION") ?: "dev"

// versionCode 由语义化版本确定性推导（dev 固定为 1）。
val semver = Regex("""^(\d+)\.(\d+)\.(\d+)$""")
val releaseVersionCode: Int = semver.find(releaseVersion)?.destructured?.let { (major, minor, patch) ->
    major.toInt() * 1_000_000 + minor.toInt() * 1_000 + patch.toInt()
} ?: 1

android {
    namespace = "com.clashedge.android"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.clashedge.android"
        minSdk = 26
        targetSdk = 34
        versionCode = releaseVersionCode
        versionName = releaseVersion

        vectorDrawables {
            useSupportLibrary = true
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
            // Signing config: injected via CI secrets (ANDROID_KEYSTORE_BASE64, ANDROID_KEY_ALIAS,
            // ANDROID_KEY_PASSWORD, ANDROID_STORE_PASSWORD). If any is missing, the release
            // artifact stays unsigned (local dev behavior).
            val storeB64 = System.getenv("ANDROID_KEYSTORE_BASE64")
            val alias = System.getenv("ANDROID_KEY_ALIAS")
            val keyPass = System.getenv("ANDROID_KEY_PASSWORD")
            val storePass = System.getenv("ANDROID_STORE_PASSWORD")
            if (storeB64 != null && alias != null && keyPass != null && storePass != null) {
                val jksFile = File(layout.buildDirectory.get().asFile, "keystore.jks")
                jksFile.writeBytes(Base64.getDecoder().decode(storeB64))
                signingConfig = signingConfigs.create("release") {
                    storeFile = jksFile
                    storePassword = storePass
                    keyAlias = alias
                    keyPassword = keyPass
                }
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
        isCoreLibraryDesugaringEnabled = true
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // Kotlin 1.9.x has no org.jetbrains.kotlin.plugin.compose (2.0+ only):
    // the compiler extension is pinned via composeOptions instead.
    composeOptions {
        kotlinCompilerExtensionVersion = libs.versions.composeCompiler.get()
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons.core)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.okhttp)
    implementation(libs.datastore.preferences)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.android)
    // Mihomo Android core (AAR / JNI) — activate once the artifact is pinned:
    // implementation(libs.mihomo.core)
    debugImplementation(libs.androidx.compose.ui.tooling)
    coreLibraryDesugaring(libs.desugar)
}
