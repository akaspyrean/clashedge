# ClashEdge Android — ProGuard / R8 rules.

# Kotlinx serialization
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.AnnotationsKt

# Mihomo JNI bindings (when the core AAR is wired in)
-keep class com.clashedge.android.mihomo.** { *; }
-keep class com.github.metacubex.** { *; }

# Mihomo core JNI classes
-keep class com.metacubex.mihomo.** { *; }
-keep class github.com.metacubex.mihomo.** { *; }

# JNI native methods
-keepclasseswithmembers class * {
    native <methods>;
}

# Gomobile generated bindings
-keep class go.** { *; }

# Keep JSON serialization for config parsing
-keepattributes Signature
-keepattributes *Annotation*
