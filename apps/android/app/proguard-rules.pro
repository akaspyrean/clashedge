# ClashEdge Android — ProGuard / R8 rules.

# Kotlinx serialization
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.AnnotationsKt

# Mihomo JNI bindings (when the core AAR is wired in)
-keep class com.clashedge.android.mihomo.** { *; }
-keep class com.github.metacubex.** { *; }
