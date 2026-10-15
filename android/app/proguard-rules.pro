# Vanillish: classes and members Rust reaches through JNI by name. R8 cannot see those calls.
-keep class io.github.klanthier.vanillish.Bridge { *; }
-keep class io.github.klanthier.vanillish.VanillishActivity { *; }
-keep class io.github.klanthier.vanillish.GamepadBridge { *; }
-keep class io.github.klanthier.vanillish.FolderImport { *; }
-keep class io.github.klanthier.vanillish.PointerBridge { *; }
-keepclasseswithmembernames class * { native <methods>; }
# GameActivity's own (its consumer rules cover Insets/WindowInsetsCompat).
-keep class com.google.androidgamesdk.** { *; }
