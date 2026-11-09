# JNI entry points are looked up by class and method name.
-keep class io.github.vickdini.iptv.Core { *; }

# Instantiated by name from the manifest by the Cast framework.
-keep class io.github.vickdini.iptv.CastOptionsProvider { *; }
