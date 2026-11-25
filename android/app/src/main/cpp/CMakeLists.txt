cmake_minimum_required(VERSION 3.18)

project(webforge_node)

# Header nodejs-mobile tidak di-commit. CI menyalin folder include/node
# dari zip nodejs-mobile ke cpp/include/node, atau meneruskan
# -DNODE_INCLUDE_DIR=<path include/node> lewat externalNativeBuild.
if(NOT DEFINED NODE_INCLUDE_DIR)
    set(NODE_INCLUDE_DIR ${CMAKE_SOURCE_DIR}/include/node)
endif()

add_library(webforge_node SHARED webforge_node.cpp)

add_library(libnode SHARED IMPORTED)
set_target_properties(libnode PROPERTIES
    IMPORTED_LOCATION ${CMAKE_SOURCE_DIR}/../jniLibs/arm64-v8a/libnode.so)

target_include_directories(webforge_node PRIVATE
    ${NODE_INCLUDE_DIR}
    ${NODE_INCLUDE_DIR}/..)

target_compile_features(webforge_node PRIVATE cxx_std_17)

target_link_libraries(webforge_node
    libnode
    log)
