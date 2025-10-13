# Ensure Gradle always prefers IPv4 when resolving dependencies to avoid network issues on IPv6-only hosts.
org.gradle.jvmargs=-Djava.net.preferIPv4Stack=true
systemProp.java.net.preferIPv4Stack=true
android.useAndroidX=true
android.enableJetifier=true
