#define DUCKDB_EXTENSION_MAIN

#include "vortex_extension.hpp"
#include "vortex.h"

using namespace duckdb;

static void LoadInternal(DatabaseInstance &db_instance) {
	vortex_init(reinterpret_cast<duckdb_database>(&db_instance));
}

/// Called when the extension is loaded by DuckDB.
/// It is responsible for registering functions and initializing state.
///
/// Specifically, the `read_vortex` table function enables reading data from
/// Vortex files in SQL queries.
void VortexExtension::Load(duckdb::DuckDB &db) {
	LoadInternal(*db.instance);
}

/// Returns the name of the Vortex extension.
///
/// It is used by DuckDB to identify the extension.
///
/// Example:
/// ```
/// LOAD vortex;
/// ```
std::string VortexExtension::Name() {
	return "vortex";
}

//! Returns the version of the Vortex extension.
std::string VortexExtension::Version() const {
	return "0.41.2";
}

#ifndef DUCKDB_EXTENSION_MAIN
#error DUCKDB_EXTENSION_MAIN not defined
#endif
