# The bindings call the Rust library through JNA, which finds its classes and callbacks by name.
-dontwarn java.awt.*
-keep class com.sun.jna.* { *; }
-keepclassmembers class * extends com.sun.jna.* { public *; }
-keep class uniffi.uriel_android.** { *; }
