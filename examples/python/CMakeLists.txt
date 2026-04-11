cmake_minimum_required(VERSION 3.20)
project(steamroom_ffi_python LANGUAGES CXX)

find_package(Python COMPONENTS Interpreter Development.Module REQUIRED)
find_package(nanobind CONFIG REQUIRED)

set(STEAMROOM_FFI_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../..")
set(STEAMROOM_FFI_LIB_DIR "${STEAMROOM_FFI_ROOT}/target/release")
set(STEAMROOM_FFI_CPP_INCLUDE "${STEAMROOM_FFI_ROOT}/crates/steamroom-ffi/bindings/cpp")

nanobind_add_module(steam_ffi_ext NB_STATIC src/bindings.cpp)

target_compile_features(steam_ffi_ext PRIVATE cxx_std_20)
target_include_directories(steam_ffi_ext PRIVATE ${STEAMROOM_FFI_CPP_INCLUDE})

if(MSVC)
    target_compile_options(steam_ffi_ext PRIVATE /Zc:__cplusplus)
endif()

if(WIN32)
    target_link_libraries(steam_ffi_ext PRIVATE
        "${STEAMROOM_FFI_LIB_DIR}/steamroom_ffi.dll.lib"
        ws2_32 bcrypt userenv ntdll
    )
elseif(APPLE)
    target_link_libraries(steam_ffi_ext PRIVATE
        "${STEAMROOM_FFI_LIB_DIR}/libsteamroom_ffi.dylib"
    )
else()
    target_link_libraries(steam_ffi_ext PRIVATE
        "${STEAMROOM_FFI_LIB_DIR}/libsteamroom_ffi.so"
    )
endif()

install(TARGETS steam_ffi_ext LIBRARY DESTINATION .)
