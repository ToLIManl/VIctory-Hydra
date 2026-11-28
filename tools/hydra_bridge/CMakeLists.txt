cmake_minimum_required(VERSION 3.24)
project(hydra_bridge VERSION 1.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")

set(SHVI_SDK_DIR "$ENV{SHVI_SDK_DIR}" CACHE PATH "ScriptHook VI sdk/include directory (pass -DSHVI_SDK_DIR=<path>)")
if(NOT SHVI_SDK_DIR)
  message(FATAL_ERROR "Set -DSHVI_SDK_DIR=<path/to/scripthook-vi/sdk/include> or SHVI_SDK_DIR env var")
endif()

add_library(hydra_bridge SHARED hydra_bridge.cpp)
target_include_directories(hydra_bridge PRIVATE "${SHVI_SDK_DIR}")
target_compile_definitions(hydra_bridge PRIVATE WIN32_LEAN_AND_MEAN NOMINMAX _CRT_SECURE_NO_WARNINGS)
target_compile_options(hydra_bridge PRIVATE /W4 /permissive- /utf-8 /bigobj)
set_target_properties(hydra_bridge PROPERTIES SUFFIX ".asi" OUTPUT_NAME "hydra_bridge")
