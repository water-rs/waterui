cmake_minimum_required(VERSION 3.22)
project(waterui_wpe C)

find_package(PkgConfig REQUIRED)
pkg_check_modules(WPE REQUIRED IMPORTED_TARGET
    wpe-webkit-2.0
    wpe-platform-2.0
    libsoup-3.0
    json-glib-1.0
    libdrm)

add_library(waterui_wpe SHARED waterui_wpe.c)
target_compile_features(waterui_wpe PRIVATE c_std_11)
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
