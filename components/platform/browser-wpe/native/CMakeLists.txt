cmake_minimum_required(VERSION 3.22)
project(waterui_wpe C)

find_package(PkgConfig REQUIRED)
pkg_check_modules(WPE REQUIRED IMPORTED_TARGET
    wpe-webkit-2.0
    wpe-platform-2.0
    libsoup-3.0
    json-glib-1.0
    libdrm)
pkg_check_modules(WPE_EXTENSION REQUIRED IMPORTED_TARGET
    wpe-web-process-extension-2.0)

add_library(waterui_wpe SHARED waterui_wpe.c)
target_compile_features(waterui_wpe PRIVATE c_std_11)
# dladdr/Dl_info in <dlfcn.h> are exposed only under _GNU_SOURCE.
target_compile_definitions(waterui_wpe PRIVATE _GNU_SOURCE)
target_compile_options(waterui_wpe PRIVATE
    -Wall
    -Wextra
    -Werror
    -Wpedantic)
target_link_libraries(waterui_wpe PRIVATE PkgConfig::WPE dl)
set_target_properties(waterui_wpe PROPERTIES
    BUILD_RPATH "$ORIGIN"
    INSTALL_RPATH "$ORIGIN")
install(TARGETS waterui_wpe LIBRARY DESTINATION lib)
install(FILES waterui_wpe.h DESTINATION include)

add_library(waterui_wpe_extension MODULE waterui_wpe_extension.c)
target_compile_features(waterui_wpe_extension PRIVATE c_std_11)
target_compile_options(waterui_wpe_extension PRIVATE
    -Wall
    -Wextra
    -Werror
    -Wpedantic)
target_link_libraries(waterui_wpe_extension PRIVATE
    PkgConfig::WPE_EXTENSION)
set_target_properties(waterui_wpe_extension PROPERTIES
    PREFIX ""
    BUILD_RPATH "$ORIGIN/../.."
    INSTALL_RPATH "$ORIGIN/../..")
install(TARGETS waterui_wpe_extension
    LIBRARY DESTINATION lib/waterui-wpe/extensions)
