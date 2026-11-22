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
// DuckDB itself builds that table with. FineType sees what it sees when loaded
// from finetype.duckdb_extension.
//
// It also registers FineType's two table macros, ft_validate and ft_profile.
// FineType's Rust registers them with a CREATE MACRO, the only way DuckDB's C
// API offers, and a CREATE writes to the database being opened. Loaded from a
// file that is one LOAD; compiled in, FineType's entry point runs on every
// database DuckDB opens, so every database opened read-write was left holding
// both macros and one opened read-only had neither. Here they go into DuckDB's
// system catalog, as a C++ extension's macros do: marked internal, written to
// no file, and present however the database was opened. The build takes
// FineType's own registration out and generates finetype_table_macros.hpp from
// FineType's SQL (move-table-macros.py beside this file).
#include "duckdb.h"
#include "duckdb/catalog/default/default_table_functions.hpp"
#include "duckdb/main/capi/capi_internal.hpp"
#include "duckdb/main/capi/extension_api.hpp"
#include "duckdb/main/extension/extension_loader.hpp"
#include "duckdb/parser/parsed_data/create_macro_info.hpp"

#include "finetype_table_macros.hpp"

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

// Register each of FineType's table macros in the system catalog of the
// database being loaded into. `get_database` hands back a wrapper DuckDB owns
// and frees when the entry point returns.
bool RegisterTableMacros(duckdb_extension_info info, duckdb_extension_access *access) {
	try {
		auto wrapper = reinterpret_cast<duckdb::DatabaseWrapper *>(access->get_database(info));
		if (!wrapper || !wrapper->database || !wrapper->database->instance) {
			access->set_error(info, "FineType's table macros: DuckDB gave no database to register them in");
			return false;
		}
		duckdb::ExtensionLoader loader(*wrapper->database->instance, "finetype");
		for (duckdb::idx_t i = 0; BRIGHTFIELD_FINETYPE_TABLE_MACROS[i].name != nullptr; i++) {
			auto macro = duckdb::DefaultTableFunctionGenerator::CreateTableMacroInfo(BRIGHTFIELD_FINETYPE_TABLE_MACROS[i]);
			loader.RegisterFunction(*macro);
		}
	} catch (std::exception &ex) {
		duckdb::ErrorData error(ex);
		auto message = "FineType's table macros could not be registered: " + error.Message();
		access->set_error(info, message.c_str());
		return false;
	}
	return true;
}

} // namespace

extern "C" bool brightfield_finetype_static_init(duckdb_extension_info info, duckdb_extension_access *access) {
	host_access = *access;
	duckdb_extension_access wrapped = *access;
	wrapped.get_api = GetAPI;
	if (!finetype_init_c_api(info, &wrapped)) {
		return false;
	}
	return RegisterTableMacros(info, access);
}
