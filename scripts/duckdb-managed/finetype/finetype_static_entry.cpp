// The entry point DuckDB calls to load FineType statically.
//
// FineType's DuckDB extension is written in Rust with duckdb-rs's
// `loadable-extension` bindings, which call DuckDB's C API through the table of
// function pointers a loaded extension asks for with `access->get_api`. DuckDB
// 2.0 loads a statically linked C API extension with `get_api` null, since it
// expects such an extension to call DuckDB's C functions directly
// (DuckDB::LoadStaticCAPIExtension in src/main/extension/extension_load.cpp).
// So FineType, as built, cannot load statically on its own.
//
// This entry point wraps FineType's own, `finetype_init_c_api`, and hands it an
// access table whose `get_api` returns the table DuckDB gives a loaded extension
// file: CreateAPIv1() from duckdb/main/capi/extension_api.hpp, the function
// DuckDB itself builds that table with. FineType's code is unchanged and sees
// what it sees when loaded from finetype.duckdb_extension.
#include "duckdb.h"
#include "duckdb/main/capi/extension_api.hpp"

#include <cstdlib>
#include <cstring>

extern "C" bool finetype_init_c_api(duckdb_extension_info info, duckdb_extension_access *access);

namespace {

duckdb_extension_access host_access;

// The C API table for the version FineType asks for. DuckDB's own get_api
// accepts any v1.x.y it supports and hands back the one v1 table; FineType asks
// for v1.2.0, its declared floor. Anything that is not a v1 version is refused,
// as DuckDB refuses it.
const void *GetAPI(duckdb_extension_info info, const char *version) {
	static duckdb_ext_api_v1 api = CreateAPIv1();
	if (!version || std::strncmp(version, "v1.", 3) != 0) {
		host_access.set_error(info, "FineType asked for a C API version other than v1.x.y");
		return nullptr;
	}
	return &api;
}

} // namespace

extern "C" bool brightfield_finetype_static_init(duckdb_extension_info info, duckdb_extension_access *access) {
	host_access = *access;
	duckdb_extension_access wrapped = *access;
	wrapped.get_api = GetAPI;
	return finetype_init_c_api(info, &wrapped);
}
