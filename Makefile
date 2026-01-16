PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXT_NAME=vortex_duckdb
EXT_CONFIG=${PROJ_DIR}extension_config.cmake
export VCPKG_FEATURE_FLAGS=-binarycaching
export VCPKG_TOOLCHAIN_PATH := ${PROJ_DIR}vcpkg/scripts/buildsystems/vcpkg.cmake

include extension-ci-tools/makefiles/duckdb_extension.Makefile
