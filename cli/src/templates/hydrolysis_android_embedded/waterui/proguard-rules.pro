# Consumer ProGuard rules for the waterui library module.
#
# Dependency crates' Kotlin sources and vendored jars staged into this module
# are reached only by name (JNI `loadClass`), which R8 cannot see. The CLI's
# android classpath staging appends a managed keep block below covering the
# package of every staged class; do not edit it by hand.
