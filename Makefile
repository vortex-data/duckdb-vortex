PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXT_NAME=vortex_duckdb
EXT_CONFIG=${PROJ_DIR}extension_config.cmake
EXT_FLAGS=-DCMAKE_OSX_DEPLOYMENT_TARGET=12.0
export MACOSX_DEPLOYMENT_TARGET=12.0
export VCPKG_FEATURE_FLAGS=-binarycaching
export VCPKG_OSX_DEPLOYMENT_TARGET=12.0
export VCPKG_TOOLCHAIN_PATH := ${PROJ_DIR}vcpkg/scripts/buildsystems/vcpkg.cmake

# This is not needed on macOS, we don't see a tls error on load there.
ifeq ($(shell uname), Linux)
    export CFLAGS=-ftls-model=global-dynamic
endif

include extension-ci-tools/makefiles/duckdb_extension.Makefile
