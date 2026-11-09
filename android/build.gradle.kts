plugins {
    id("com.android.application") version "9.4.1" apply false
    // AGP 9 compiles Kotlin itself (built-in Kotlin). Declaring the Kotlin
    // plugin here only pins the Kotlin version; it is not applied.
    id("org.jetbrains.kotlin.android") version "2.4.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.21" apply false
}
