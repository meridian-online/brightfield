# The vcpkg triplet the managed edition's DuckDB builds its C libraries with
# (GDAL, PROJ, GEOS, curl, OpenSSL and the rest the extensions need).
#
# vcpkg's own arm64-osx builds each library twice, release and debug, and for the
# macOS version of the machine building it. This builds release only, which is
# all the library links, and for macOS 11, the oldest macOS DuckDB's own release
# library and shell run on, so no C library in the app needs a newer macOS than
# DuckDB does.
set(VCPKG_TARGET_ARCHITECTURE arm64)
set(VCPKG_CRT_LINKAGE dynamic)
set(VCPKG_LIBRARY_LINKAGE static)
set(VCPKG_CMAKE_SYSTEM_NAME Darwin)
set(VCPKG_OSX_ARCHITECTURES arm64)
set(VCPKG_OSX_DEPLOYMENT_TARGET 11.0)
set(VCPKG_BUILD_TYPE release)
