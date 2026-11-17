// Describes FineType to duckdb_register_static_extension, as the describe
// function DuckDB's build generates for a C API extension does
// (extension/loader/extension_describe.c.in), except that its entry point is
// brightfield_finetype_static_init in finetype_static_entry.cpp, which wraps
// FineType's own. BRIGHTFIELD_FINETYPE_VERSION is FineType's release tag.
#include "duckdb_static_extension.h"

int brightfield_finetype_static_init(void *info, void *access);

int32_t duckdb_extension_finetype_describe(duckdb_extension_descriptor *descriptor) {
	if (descriptor->version < 1) {
		descriptor->set_error(descriptor, "extension finetype needs descriptor layout 1");
		return 1;
	}
	descriptor->version = 1;
	descriptor->name = "finetype";
	descriptor->extension_version = BRIGHTFIELD_FINETYPE_VERSION;
	// The C API version FineType's entry point asks for (MIN_DUCKDB_VERSION in
	// crates/finetype-duckdb/src/lib.rs of meridian-online/finetype).
	descriptor->api_version = "v1.2.0";
	descriptor->entry_capi_v1 = (void (*)(void))brightfield_finetype_static_init;
	return 0;
}
