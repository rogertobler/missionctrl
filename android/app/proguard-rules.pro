# R8 rules for the release build, on top of proguard-android-optimize.txt.
#
# The Rust core is reached through UniFFI's Kotlin bindings, which call into libmc_ffi.so with JNA.
# JNA works by reflection and from native code (libjnidispatch.so looks up its own classes, fields
# and methods by name), and the JNA aar ships no keep rules of its own:
# - Native.register() binds `external fun` declarations by method name to the library's symbols
#   (the default rule keeps the names of the native methods that remain after shrinking),
# - Structure subclasses are read and written field by field, in the order named by
#   @Structure.FieldOrder, and created through their no-argument constructors,
# - callbacks from Rust arrive through interfaces that extend com.sun.jna.Callback.

# JNA itself; it refers to java.awt, which Android does not have.
-dontwarn java.awt.**
-keep class com.sun.jna.** { *; }

# Generated bindings: structures (fields, constructors, ByValue/ByReference variants) and callbacks.
-keep class * extends com.sun.jna.Structure { *; }
-keep class * implements com.sun.jna.Callback { *; }
