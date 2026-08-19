# Vortex DuckDB

If you encounter a bug or you have a feature request, please open an issue
or discussion in https://github.com/vortex-data/vortex.

## Building

Run:

```sh
make
```

You will get a release build with the binaries being:

```sh
./build/release/duckdb
./build/release/extension/vortex_duckdb/vortex_duckdb.duckdb_extension
```

- `duckdb` includes Vortex extension linkes statically.
- `vortex_duckdb.duckdb_extension` is the extension as distributed by Duckdb.


### Writing a file

```sql
COPY (SELECT * FROM generate_series(0, 4)) TO 'file.vortex';
```

### Reading a file

```sql
SELECT * FROM 'file.vortex';
```

## Building shared Vortex library

```sh
~/duckdb-vortex make EXT_FLAGS='-DUSE_SHARED_VORTEX=1' reldebug -j
```

## Changing Vortex version

The Vortex version is defined in `vortex-extension/Cargo.toml`. It can be a git commit, tag, branch or a local path:

```toml
vortex-duckdb = { path = "<path/to/vortex/vortex-duckdb>"}
```

See the Cargo docs for [git](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#specifying-dependencies-from-git-repositories) or [path](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#specifying-path-dependencies) dependencies for full details.
