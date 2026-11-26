# Rust calls these classes' methods and native functions by name, and
# Android starts them from the manifest. Shrinking must not rename or drop them.
-keep class dev.launcherrs.gallery.** { *; }
