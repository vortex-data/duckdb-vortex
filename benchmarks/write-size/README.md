# Write throughput and file size: Vortex vs (compressed) Parquet

A reproduction of [vortex-data/vortex#5861](https://github.com/vortex-data/vortex/issues/5861)
("Write performance for small arrow RecordBatch 20x slower than parquet"), extended to answer a
second question the original benchmark leaves open: the Parquet side of that comparison is written
with `WriterProperties::builder().build()`, and in arrow-rs `DEFAULT_COMPRESSION` is
`Compression::UNCOMPRESSED`. So the benchmark times Vortex *with* compression against Parquet
*without* it. How do the two compare on size and speed once Parquet is actually compressed?

## Running it

```sh
cargo run --release -- 10 1000 10000 100000 1000000   # size/throughput matrix
cargo run --release -- probe                          # where the small-write fixed cost goes
```

The data generator is copied verbatim from the issue: 20 columns (3 `Int64`, 17 `Utf8`) of
synthetic Kubernetes log records, chunked into 8192-row `RecordBatch`es.

Writers measured:

| writer | what it is |
| --- | --- |
| `parquet_uncompressed` | `WriterProperties::builder().build()` — exactly what the issue benchmarks |
| `parquet_snappy` / `parquet_zstd{1,3,9}` | same, with block compression enabled |
| `vortex_async` / `vortex_blocking` | the issue's two Vortex paths, default write options |
| `vortex_compact` | `BtrBlocksCompressorBuilder::default().with_compact()` — adds the Zstd/Pco schemes |
| `vortex_bigblock` | default schemes, `row_block_size` 65536 and 16 MiB data blocks |
| `vortex_compact_bigblock` | both of the above |

Each writer is warmed up once, then run for at least 5 iterations and at least 3 seconds; the
median is reported. `file bytes` is the size of the file the last iteration produced, and
`vs arrow` is that against the summed `RecordBatch::get_array_memory_size()`.

## Results

Measured on 4 vCPU Intel Xeon @ 2.80GHz, 15 GB RAM, against vortex rev
`68e2aee0afddc8eb0f3b216a7580c6743ab509f0` (2026-08-27). Absolute times are slower than the
issue reporter's machine — they measured 285 µs for `parquet/10` where this box measures 400 µs —
so compare ratios, not wall clock.

```

=== 10 rows, 1 batch(es), 7368 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed      400.633µs    343.016µs           9864         17.5      0.75x
parquet_snappy            411.796µs    305.633µs           7710         17.1      0.96x
parquet_zstd1               1.506ms      1.403ms           7667          4.7      0.96x
parquet_zstd3               1.538ms      1.429ms           7662          4.6      0.96x
parquet_zstd9               1.571ms      1.457ms           7606          4.5      0.97x
vortex_async               33.906ms     31.626ms          24400          0.2      0.30x
vortex_blocking            20.710ms     18.986ms          24400          0.3      0.30x
vortex_compact             23.607ms     21.781ms          19448          0.3      0.38x
vortex_bigblock            21.050ms     19.366ms          24400          0.3      0.30x
vortex_compact_bigblock     23.705ms     21.423ms          19448          0.3      0.38x

=== 1000 rows, 1 batch(es), 549528 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed        1.532ms      1.338ms         232718        342.1      2.36x
parquet_snappy              1.528ms      1.350ms          52238        342.9     10.52x
parquet_zstd1               2.781ms      2.560ms          23989        188.5     22.91x
parquet_zstd3               3.042ms      2.649ms          24132        172.3     22.77x
parquet_zstd9               6.188ms      5.714ms          23467         84.7     23.42x
vortex_async               82.988ms     73.503ms          86440          6.3      6.36x
vortex_blocking            61.264ms     55.240ms          86440          8.6      6.36x
vortex_compact             69.064ms     65.378ms          25240          7.6     21.77x
vortex_bigblock            57.674ms     53.225ms          86440          9.1      6.36x
vortex_compact_bigblock     65.937ms     63.865ms          25240          7.9     21.77x

=== 10000 rows, 2 batch(es), 5473392 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed       11.641ms      9.965ms        1912864        448.4      2.86x
parquet_snappy             13.492ms     11.892ms         329845        386.9     16.59x
parquet_zstd1              16.704ms     14.795ms         121875        312.5     44.91x
parquet_zstd3              18.188ms     15.742ms         131191        287.0     41.72x
parquet_zstd9              34.981ms     32.227ms         114877        149.2     47.65x
vortex_async              109.883ms    100.856ms         599360         47.5      9.13x
vortex_blocking            89.973ms     82.298ms         599360         58.0      9.13x
vortex_compact            110.830ms    104.535ms          60456         47.1     90.54x
vortex_bigblock            95.487ms     88.689ms         598040         54.7      9.15x
vortex_compact_bigblock    107.655ms    101.489ms          60136         48.5     91.02x

=== 100000 rows, 13 batch(es), 53726968 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed      133.743ms    122.588ms       18525617        383.1      2.90x
parquet_snappy            125.661ms    118.770ms        2758568        407.7     19.48x
parquet_zstd1             134.590ms    120.022ms         761412        380.7     70.56x
parquet_zstd3             154.048ms    146.078ms         781090        332.6     68.78x
parquet_zstd9             355.365ms    347.326ms         690599        144.2     77.80x
vortex_async              353.507ms    339.157ms        5058784        144.9     10.62x
vortex_blocking           324.704ms    300.414ms        5058784        157.8     10.62x
vortex_compact            367.387ms    364.888ms         417712        139.5    128.62x
vortex_bigblock           326.991ms    303.466ms        5174584        156.7     10.38x
vortex_compact_bigblock    313.174ms    286.783ms         505984        163.6    106.18x

=== 1000000 rows, 123 batch(es), 536406088 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed         1.019s    877.138ms      183752650        501.8      2.92x
parquet_snappy            841.625ms    829.260ms       24410331        607.8     21.97x
parquet_zstd1             888.766ms    856.472ms        4538650        575.6    118.19x
parquet_zstd3                1.055s       1.042s        4309735        484.7    124.46x
parquet_zstd9                2.660s       2.607s        3899632        192.3    137.55x
vortex_async                 3.008s       2.750s       55959392        170.1      9.59x
vortex_blocking              2.654s       2.500s       55959392        192.8      9.59x
vortex_compact               3.210s       3.058s        3919000        159.4    136.87x
vortex_bigblock              2.162s       2.145s       58960040        236.6      9.10x
vortex_compact_bigblock       2.214s       2.202s        3859856        231.0    138.97x
```

## Is #5861 still a problem?

Yes, and the shape of it is unchanged. Vortex is slower than uncompressed Parquet at every size,
but the multiple collapses as rows are added:

| rows | parquet uncompressed | vortex blocking | ratio |
| ---: | ---: | ---: | ---: |
| 10 | 0.40 ms | 20.7 ms | 52x |
| 1,000 | 1.53 ms | 61.3 ms | 40x |
| 10,000 | 11.6 ms | 90.0 ms | 7.7x |
| 100,000 | 134 ms | 325 ms | 2.4x |
| 1,000,000 | 1.02 s | 2.65 s | 2.6x |

The blocking path is consistently faster than the async one, as the reporter also found, so the
gap is not a Tokio artifact.

## Where the small-write cost goes

It is a fixed per-column cost, not a per-row one. A 10-row write costs roughly 1 ms per column:

```

=== fixed-cost probe: 10 rows, varying column count ===
case                               median          min   file bytes
1 cols                          535.551µs    381.915µs         2180
  (1 cols, parquet)             145.922µs     71.635µs          583
2 cols                            2.109ms      1.935ms         3740
  (2 cols, parquet)             178.461µs     93.627µs         1893
5 cols                            6.262ms      5.483ms         7260
  (5 cols, parquet)             160.122µs    113.460µs         3737
10 cols                          11.448ms     10.508ms        12956
  (10 cols, parquet)            306.869µs    226.739µs         5701
20 cols                          21.439ms     19.136ms        24400
  (20 cols, parquet)            444.175µs    312.814µs         9864

=== 10 rows, 20 cols: what costs the ~1ms/column? ===
empty compressor                  4.278ms      3.643ms        25104
default compressor               22.280ms     20.138ms        24400
compact compressor               24.255ms     21.475ms        19448

=== 10 rows, 20 cols, no file statistics ===
no file stats                    20.770ms     19.179ms        22752
```

Replacing the compressor with `BtrBlocksCompressorBuilder::empty()` takes the 20-column, 10-row
write from 22.3 ms to 4.3 ms, so about 80% of the floor is scheme search evaluating candidate
encodings against 10-element arrays. Disabling file statistics changes nothing. The remaining
~4 ms of layout, footer and IO machinery is still an order of magnitude above Parquet's 0.44 ms,
but scheme search is where the bulk of it is.

Any fix probably wants a size threshold below which scheme search is skipped in favour of a fixed
cheap encoding, since on a 10-element array the search cannot pay for itself.

## Are we smaller than Parquet once Parquet is compressed?

**With default write options, no — we are substantially larger.** At 1M rows:

| writer | file bytes | vs default vortex |
| --- | ---: | ---: |
| parquet uncompressed | 183,752,650 | 3.3x larger |
| parquet snappy | 24,410,331 | 2.3x smaller |
| parquet zstd(1) | 4,538,650 | 12.3x smaller |
| parquet zstd(9) | 3,899,632 | 14.4x smaller |
| vortex (default) | 55,959,392 | — |

Default Vortex only beats *uncompressed* Parquet. It loses to snappy by 2.3x and to zstd(1) by
12x.

The cause is not block sizing — `vortex_bigblock` writes 64k-row blocks into 16 MiB compression
units and lands within 5% of the default. It is the scheme list: `ALL_SCHEMES` in
`vortex-btrblocks` contains no general-purpose compressor at all. Zstd and Pco are feature-gated
and join the compressor only through `BtrBlocksCompressorBuilder::with_compact()`. So the default
comparison is lightweight encodings against lightweight encodings *plus an entropy coder*, and the
entropy coder wins on text.

**With `with_compact()`, the ordering inverts at every size:**

| rows | parquet zstd(1) | parquet zstd(9) | vortex compact | vortex compact + big blocks |
| ---: | ---: | ---: | ---: | ---: |
| 1,000 | 23,989 | 23,467 | 25,240 | 25,240 |
| 10,000 | 121,875 | 114,877 | **60,456** | **60,136** |
| 100,000 | 761,412 | 690,599 | **417,712** | 505,984 |
| 1,000,000 | 4,538,650 | 3,899,632 | 3,919,000 | **3,859,856** |

Compact Vortex is 2x smaller than zstd(1) Parquet at 10k and 100k rows, and at 1M rows it matches
zstd(9) while writing faster than it (2.21 s vs 2.66 s for the big-block variant). It is still
~2.5x slower to write than zstd(1), which is the setting most Parquet writers actually use.

Note that `vortex_compact_bigblock` beats `vortex_compact` at 1M rows but loses to it at 100k
(505,984 vs 417,712) — larger blocks change which schemes get selected, not just how much data
each one sees, so the bigger unit is not uniformly better.

## Caveat on the data

The generator is unusually kind to general-purpose compression. Consecutive `log` values differ
only in an embedded integer, so zstd finds long cross-row matches and reaches a 118x ratio on the
1M-row case. Real log data will not compress like that, and the absolute ratios here should be
read as an upper bound on zstd's advantage. The ordering conclusion is robust to that, though,
because it follows from the default scheme list rather than from the data.

## Relevance to this repo

`vortex-duckdb/src/copy.rs` calls plain `SESSION.write_options()` with no strategy override, and
the COPY function exposes no option to change it, so `COPY … TO 'x.vortex'` always writes the
non-compact layout. The Python bindings expose the choice as
`vortex.io.VortexWriteOptions.compact()` and `vortex-tui convert` exposes it as
`--strategy compact`; DuckDB has no equivalent. On text-heavy tables that makes
`COPY … TO 'x.vortex'` produce a much larger file than
`COPY … TO 'x.parquet' (COMPRESSION zstd)`, with no way to ask for anything else.
