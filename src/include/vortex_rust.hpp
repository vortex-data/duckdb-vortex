#pragma once

extern "C" {

void vortex_init_rust(void *db);
const char *vortex_version_rust();
const char *vortex_extension_version_rust();

}
