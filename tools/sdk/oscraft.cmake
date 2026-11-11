# CMake toolchain file for OsCraft: `cmake -DCMAKE_TOOLCHAIN_FILE=<sdk>/oscraft.cmake ...`.
# OsCraft programs are FreeBSD programs for aarch64 built against this
# SDK's sysroot with the host's clang and lld.
get_filename_component(OSCRAFT_SDK "${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)
set(CMAKE_SYSTEM_NAME FreeBSD)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_SYSROOT "${OSCRAFT_SDK}/sysroot")
set(CMAKE_C_COMPILER "${OSCRAFT_SDK}/bin/oscraft-cc")
set(CMAKE_CXX_COMPILER "${OSCRAFT_SDK}/bin/oscraft-c++")
set(CMAKE_ASM_COMPILER "${OSCRAFT_SDK}/bin/oscraft-cc")
set(CMAKE_C_COMPILER_TARGET aarch64-unknown-freebsd15.2)
set(CMAKE_CXX_COMPILER_TARGET aarch64-unknown-freebsd15.2)
set(CMAKE_FIND_ROOT_PATH "${CMAKE_SYSROOT}")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
