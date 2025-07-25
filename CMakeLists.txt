cmake_minimum_required(VERSION 3.22)

set(TARGET_NAME vortex)
project(${TARGET_NAME}_project)

set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
set(CMAKE_CXX_STANDARD 17)

include(FetchContent)
FetchContent_Declare(
        Corrosion
        GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
        GIT_TAG v0.5.2
)

FetchContent_MakeAvailable(Corrosion)

if (APPLE)
    find_library(SECURITY_FRAMEWORK Security)
    find_library(CORE_FOUNDATION_FRAMEWORK CoreFoundation)
endif ()

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

corrosion_import_crate(MANIFEST_PATH vortex/Cargo.toml
        CRATES vortex-duckdb
        CRATE_TYPES staticlib
        FLAGS --crate-type=staticlib
)

include_directories(src/include vortex/vortex-duckdb/include)

set(EXTENSION_NAME ${TARGET_NAME}_extension)
set(LOADABLE_EXTENSION_NAME ${TARGET_NAME}_loadable_extension)

build_static_extension(${TARGET_NAME} src/vortex_extension.cpp)
build_loadable_extension(${TARGET_NAME} -warnings src/vortex_extension.cpp)

target_link_libraries(${EXTENSION_NAME}
        vortex_duckdb-static
        ${SECURITY_FRAMEWORK}
        ${CORE_FOUNDATION_FRAMEWORK}
)

install(
        TARGETS ${EXTENSION_NAME}
        EXPORT "${DUCKDB_EXPORT_SET}"
        LIBRARY DESTINATION "${INSTALL_LIB_DIR}"
        ARCHIVE DESTINATION "${INSTALL_LIB_DIR}"
)
