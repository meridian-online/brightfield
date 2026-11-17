# FineType as a statically linked DuckDB extension.
#
# FineType's DuckDB extension is a Rust crate (crates/finetype-duckdb in
# meridian-online/finetype) written against DuckDB's C extension API, with the
# entry point `finetype_init_c_api`. scripts/build-duckdb.sh builds that crate as
# a static library and passes its path in BRIGHTFIELD_FINETYPE_STATICLIB.
#
# This makes `finetype_extension`, the archive DuckDB's build links into the
# library, from that static library, an entry point that lets FineType's
# bindings find DuckDB's C API (finetype_static_entry.cpp), and the describe
# function DuckDB 2.0 registers a statically linked extension through
# (finetype_describe.c). It does what build_static_extension_capi in DuckDB's
# extension/extension_build_tools.cmake does, with that entry point in place of
# FineType's own.
cmake_minimum_required(VERSION 3.5...3.29)
project(FinetypeExtension C CXX)

if(NOT DEFINED BRIGHTFIELD_FINETYPE_STATICLIB OR NOT EXISTS "${BRIGHTFIELD_FINETYPE_STATICLIB}")
  message(FATAL_ERROR "BRIGHTFIELD_FINETYPE_STATICLIB must name FineType's built static library")
endif()
if(NOT DEFINED BRIGHTFIELD_FINETYPE_VERSION)
  message(FATAL_ERROR "BRIGHTFIELD_FINETYPE_VERSION must name FineType's release tag")
endif()

add_library(finetype_extension STATIC
            ${CMAKE_CURRENT_SOURCE_DIR}/finetype_static_entry.cpp
            ${CMAKE_CURRENT_SOURCE_DIR}/finetype_describe.c)
target_compile_definitions(finetype_extension PRIVATE
                           BRIGHTFIELD_FINETYPE_VERSION="${BRIGHTFIELD_FINETYPE_VERSION}")
target_link_libraries(finetype_extension
  "$<$<NOT:$<BOOL:$<TARGET_PROPERTY:DUCKDB_DYNAMIC_LOADABLE_EXTENSION>>>:duckdb_static>"
  "${BRIGHTFIELD_FINETYPE_STATICLIB}")
set_property(TARGET finetype_extension PROPERTY DUCKDB_EXTENSION_KIND CAPI)
if(APPLE)
  # What the Rust standard library and FineType's HTTP client (hf-hub, through
  # native-tls) take from the system, which a static library does not record.
  target_link_libraries(finetype_extension "-framework CoreFoundation" "-framework Security"
                        "-framework SystemConfiguration" iconv)
endif()
