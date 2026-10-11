package dev.waterui.hydrolysis

/**
 * Marks a member the native library calls by name through JNI.
 *
 * This library's consumer keep rule keeps every member carrying the
 * annotation, so R8 cannot strip or rename the JNI contract when it shrinks
 * an app embedding the host. BINARY retention keeps the marker in the class
 * file where R8 reads it; it is never observed at runtime.
 */
@Retention(AnnotationRetention.BINARY)
@Target(AnnotationTarget.FUNCTION, AnnotationTarget.CONSTRUCTOR)
annotation class CalledFromNative
