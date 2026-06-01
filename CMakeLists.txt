cmake_minimum_required(VERSION 3.22)

# Use extraction time for timestamps of files fetched via `FetchContent`.
cmake_policy(SET CMP0135 NEW)

set(TARGET_NAME vortex)
project(${TARGET_NAME}_project)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# DUCKDB_VERSION is passed down as an env var to Vortex which
# in turn builds against the respective version. It is mandatory 
# that the version is set to prevent implicit ABI breaks.
if(NOT DEFINED DUCKDB_VERSION OR "${DUCKDB_VERSION}" STREQUAL "")
    message(FATAL_ERROR "DUCKDB_VERSION must be set by DuckDB's CMake build")
endif()

option(USE_SHARED_VORTEX OFF)
set(VORTEX_RUST_TARGET_NAME "vortex_duckdb")
if (USE_SHARED_VORTEX)
    set(VORTEX_LIB_TYPE "cdylib")
    set(VORTEX_LIB_NAME "${VORTEX_RUST_TARGET_NAME}-shared")
else()
    set(VORTEX_LIB_TYPE "staticlib")
    set(VORTEX_LIB_NAME "${VORTEX_RUST_TARGET_NAME}-static")
endif()
message("Using vortex lib type '${VORTEX_LIB_TYPE}'")

include(FetchContent)
FetchContent_Declare(
    Corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG v0.5.2
)
FetchContent_MakeAvailable(Corrosion)

if(APPLE)
    find_library(SECURITY_FRAMEWORK Security)
    find_library(CORE_FOUNDATION_FRAMEWORK CoreFoundation)
endif()

# Install clang and libclang for Linux Docker builds. macOS has both of these pre-installed.
if(EXISTS "/.dockerenv")
    find_program(YUM_EXEC yum)
    if(YUM_EXEC AND NOT INSTALL_SUCCESS)
        execute_process(
            COMMAND yum install -y clang-devel clang
            RESULT_VARIABLE YUM_RESULT
        )
        if(YUM_RESULT EQUAL 0)
            set(INSTALL_SUCCESS TRUE)
        endif()
    endif()

    find_program(APK_EXEC apk)
    if(APK_EXEC AND NOT INSTALL_SUCCESS)
        execute_process(
            COMMAND apk add --no-cache clang-dev clang
            RESULT_VARIABLE APK_RESULT
        )
        if(APK_RESULT EQUAL 0)
            set(INSTALL_SUCCESS TRUE)
        endif()
    endif()
endif()

corrosion_import_crate(
    MANIFEST_PATH vortex/Cargo.toml
    CRATES vortex-duckdb
    CRATE_TYPES ${VORTEX_LIB_TYPE}
    FLAGS --crate-type=${VORTEX_LIB_TYPE}
)

# Forward the DuckDB version passed via CMake as an env var.
message(STATUS "Forwarding DUCKDB_VERSION=${DUCKDB_VERSION} to Cargo")
corrosion_set_env_vars(
    ${VORTEX_RUST_TARGET_NAME}
    "DUCKDB_VERSION=${DUCKDB_VERSION}"
)

# On musl (Alpine) the vortex crate is built as a staticlib and folded into the
# loadable extension, which is a shared object. Rust's x86_64-unknown-linux-musl
# target defaults to a static CRT and non-PIC codegen, so the archive fails to
# link into a shared object (relocation R_X86_64_32 ... can not be used when
# making a shared object). Build it position-independent against a dynamic CRT.
# Scoped to musl via the loader path so glibc/macOS builds are unaffected.
if(EXISTS "/lib/ld-musl-x86_64.so.1")
    message(STATUS "musl libc detected: building vortex crate as PIC with dynamic CRT")
    corrosion_add_target_rustflags(
        ${VORTEX_RUST_TARGET_NAME}
        "-Ctarget-feature=-crt-static"
        "-Crelocation-model=pic"
    )
endif()

include_directories(src/include vortex/vortex-duckdb/include)

set(EXTENSION_NAME ${TARGET_NAME}_extension)
set(LOADABLE_EXTENSION_NAME ${TARGET_NAME}_loadable_extension)

build_static_extension(${TARGET_NAME} src/vortex_extension.cpp)
build_loadable_extension(${TARGET_NAME} "" src/vortex_extension.cpp)

set(VORTEX_WARNING_FLAGS -Wall -Wextra -Wpedantic)

target_compile_options(${EXTENSION_NAME} PRIVATE ${VORTEX_WARNING_FLAGS})
target_link_libraries(${EXTENSION_NAME}
    ${VORTEX_LIB_NAME}
    ${SECURITY_FRAMEWORK}
    ${CORE_FOUNDATION_FRAMEWORK}
)

target_compile_options(${LOADABLE_EXTENSION_NAME} PRIVATE ${VORTEX_WARNING_FLAGS})
target_link_libraries(${LOADABLE_EXTENSION_NAME}
    ${VORTEX_LIB_NAME}
    ${SECURITY_FRAMEWORK}
    ${CORE_FOUNDATION_FRAMEWORK}
)

install(
    TARGETS ${EXTENSION_NAME}
    EXPORT "${DUCKDB_EXPORT_SET}"
    LIBRARY DESTINATION "${INSTALL_LIB_DIR}"
    ARCHIVE DESTINATION "${INSTALL_LIB_DIR}"
)
