# Auto-generated wrapper toolchain for WaterUI Android builds
# Sets ANDROID_ABI before including the NDK toolchain to fix cmake-rs cross-compilation
set(ANDROID_ABI "{abi}")
set(ANDROID_PLATFORM "android-{api_level}")
include("{ndk_toolchain}")

# Ensure ASM compiler is an absolute path. Some transitive CMake projects
# (e.g. aws-lc-sys) reject non-absolute compiler names for ASM.
set(CMAKE_ASM_COMPILER "{asm_compiler}" CACHE FILEPATH "" FORCE)
