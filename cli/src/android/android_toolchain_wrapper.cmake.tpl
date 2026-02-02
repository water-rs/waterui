# Auto-generated wrapper toolchain for WaterUI Android builds
# Sets ANDROID_ABI before including the NDK toolchain to fix cmake-rs cross-compilation
set(ANDROID_ABI "{abi}")
set(ANDROID_PLATFORM "android-{api_level}")
include("{ndk_toolchain}")

