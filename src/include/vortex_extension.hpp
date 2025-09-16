#pragma once

#include "duckdb.hpp"

// The entry point class API can't be scoped to the vortex namespace.

class VortexExtension : public duckdb::Extension {
public:
	void Load(duckdb::ExtensionLoader &loader) override;
	std::string Name() override;
	std::string Version() const override;
};
