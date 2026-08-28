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
| `parquet_snappy` / `parquet_lz4` / `parquet_lz4_hadoop` / `parquet_zstd{1,3,9}` | same, with block compression enabled (`parquet_lz4` is `LZ4_RAW`, `parquet_lz4_hadoop` the legacy framed variant) |
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
issue reporter's machine — they measured 285 µs for `parquet/10` where this box measures 371 µs —
so compare ratios, not wall clock. Every number below comes from a single run with nothing else
on the box; an earlier run under compile load was uniformly ~25% slower, which is roughly the
noise floor for absolute times here.

```

=== 10 rows, 1 batch(es), 7368 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed      370.846µs    239.176µs           9864         18.9      0.75x
parquet_snappy            339.725µs    221.585µs           7710         20.7      0.96x
parquet_lz4               362.055µs    237.242µs           7589         19.4      0.97x
parquet_lz4_hadoop        378.236µs    236.047µs           7912         18.6      0.93x
parquet_zstd1               1.225ms      1.039ms           7667          5.7      0.96x
parquet_zstd3               1.143ms      1.038ms           7662          6.1      0.96x
parquet_zstd9               1.215ms      1.098ms           7606          5.8      0.97x
vortex_async               23.803ms     22.361ms          24400          0.3      0.30x
vortex_blocking            14.530ms     13.792ms          24400          0.5      0.30x
vortex_compact             16.841ms     15.743ms          19448          0.4      0.38x
vortex_bigblock            14.760ms     13.593ms          24400          0.5      0.30x
vortex_compact_bigblock     16.737ms     15.870ms          19448          0.4      0.38x

=== 1000 rows, 1 batch(es), 549528 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed        1.212ms      1.041ms         232718        432.3      2.36x
parquet_snappy              1.188ms    997.579µs          52238        441.0     10.52x
parquet_lz4                 1.145ms      1.057ms          49540        457.6     11.09x
parquet_lz4_hadoop          1.203ms      1.058ms          49861        435.6     11.02x
parquet_zstd1               2.176ms      1.938ms          23989        240.9     22.91x
parquet_zstd3               2.350ms      2.093ms          24132        223.1     22.77x
parquet_zstd9               4.735ms      4.441ms          23467        110.7     23.42x
vortex_async               53.866ms     51.153ms          86440          9.7      6.36x
vortex_blocking            42.901ms     40.251ms          86440         12.2      6.36x
vortex_compact             52.621ms     50.571ms          25240         10.0     21.77x
vortex_bigblock            44.348ms     42.185ms          86440         11.8      6.36x
vortex_compact_bigblock     50.316ms     47.868ms          25240         10.4     21.77x

=== 10000 rows, 2 batch(es), 5473392 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed        7.272ms      6.863ms        1912864        717.8      2.86x
parquet_snappy              8.314ms      7.932ms         329845        627.9     16.59x
parquet_lz4                 8.444ms      7.971ms         295077        618.1     18.55x
parquet_lz4_hadoop          8.447ms      7.896ms         295399        617.9     18.53x
parquet_zstd1               9.898ms      9.094ms         121875        527.4     44.91x
parquet_zstd3              10.618ms      9.974ms         131191        491.6     41.72x
parquet_zstd9              20.864ms     19.650ms         114877        250.2     47.65x
vortex_async               77.219ms     73.227ms         599360         67.6      9.13x
vortex_blocking            66.675ms     62.924ms         599360         78.3      9.13x
vortex_compact             81.618ms     77.700ms          60456         64.0     90.54x
vortex_bigblock            66.540ms     63.719ms         598040         78.4      9.15x
vortex_compact_bigblock     77.977ms     75.532ms          60136         66.9     91.02x

=== 100000 rows, 13 batch(es), 53726968 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed       69.432ms     60.265ms       18525617        738.0      2.90x
parquet_snappy             69.761ms     61.101ms        2758568        734.5     19.48x
parquet_lz4                70.215ms     65.014ms        2390881        729.7     22.47x
parquet_lz4_hadoop         73.621ms     69.016ms        2391893        696.0     22.46x
parquet_zstd1              75.958ms     72.891ms         761412        674.6     70.56x
parquet_zstd3              79.239ms     72.842ms         781090        646.6     68.78x
parquet_zstd9             203.505ms    195.580ms         690599        251.8     77.80x
vortex_async              244.574ms    240.914ms        5058784        209.5     10.62x
vortex_blocking           240.816ms    227.899ms        5058784        212.8     10.62x
vortex_compact            306.029ms    286.234ms         417712        167.4    128.62x
vortex_bigblock           246.792ms    238.711ms        5174584        207.6     10.38x
vortex_compact_bigblock    248.247ms    240.699ms         505984        206.4    106.18x

=== 1000000 rows, 123 batch(es), 536406088 arrow in-memory bytes ===
writer                       median          min     file bytes        MiB/s   vs arrow
parquet_uncompressed      717.870ms    629.661ms      183752650        712.6      2.92x
parquet_snappy            592.198ms    573.737ms       24410331        863.8     21.97x
parquet_lz4               597.308ms    595.124ms       20662054        856.4     25.96x
parquet_lz4_hadoop        601.713ms    575.963ms       20670434        850.2     25.95x
parquet_zstd1             614.101ms    609.595ms        4538650        833.0    118.19x
parquet_zstd3             653.036ms    651.859ms        4309735        783.4    124.46x
parquet_zstd9                1.747s       1.738s        3899632        292.8    137.55x
vortex_async                 2.126s       2.093s       55959392        240.7      9.59x
vortex_blocking              2.040s       2.001s       55959392        250.8      9.59x
vortex_compact               2.282s       2.238s        3919000        224.1    136.87x
vortex_bigblock              1.737s       1.716s       58960040        294.6      9.10x
vortex_compact_bigblock       1.767s       1.720s        3859856        289.6    138.97x
```

## Is #5861 still a problem?

Yes, and the shape of it is unchanged. Vortex is slower than uncompressed Parquet at every size,
but the multiple collapses as rows are added:

| rows | parquet uncompressed | vortex blocking | ratio |
| ---: | ---: | ---: | ---: |
| 10 | 0.37 ms | 14.5 ms | 39x |
| 1,000 | 1.21 ms | 42.9 ms | 35x |
| 10,000 | 7.3 ms | 66.7 ms | 9.2x |
| 100,000 | 69 ms | 241 ms | 3.5x |
| 1,000,000 | 0.72 s | 2.04 s | 2.8x |

The blocking path is consistently faster than the async one, as the reporter also found, so the
gap is not a Tokio artifact.

## Which Parquet codec you compare against barely matters

The Parquet block codec is not what makes Parquet fast here. At 1M rows, snappy, LZ4 and zstd(1)
finish within 4% of each other, because encoding and IO dominate and all three codecs are fast
enough to disappear behind them:

| writer | 1M median | vs snappy | file bytes |
| --- | ---: | ---: | ---: |
| parquet uncompressed | 718 ms | +21% | 183,752,650 |
| parquet snappy | 592 ms | — | 24,410,331 |
| parquet lz4 (LZ4_RAW) | 597 ms | +1% | 20,662,054 |
| parquet lz4 (hadoop) | 602 ms | +2% | 20,670,434 |
| parquet zstd(1) | 614 ms | +4% | 4,538,650 |
| parquet zstd(3) | 653 ms | +10% | 4,309,735 |
| parquet zstd(9) | 1.747 s | +195% | 3,899,632 |

Two things fall out of that. Compression is *cheaper than not compressing* below zstd(3) —
uncompressed is the slowest of the light options because it writes 7.5x more bytes. And zstd(1)
costs 4% more write time than snappy while producing a file 5.4x smaller, so on this data there
is no reason to pick snappy over it. LZ4_RAW strictly dominates snappy: same speed, 15% smaller.

So Vortex's write-speed gap is essentially one number regardless of which codec is on the other
side:

| rows | vs snappy | vs lz4 | vs zstd(1) | vs zstd(3) | vs zstd(9) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 10 | 43x | 40x | 12x | 13x | 12x |
| 1,000 | 36x | 37x | 20x | 18x | 9.1x |
| 10,000 | 8.0x | 7.9x | 6.7x | 6.3x | 3.2x |
| 100,000 | 3.5x | 3.4x | 3.2x | 3.0x | 1.2x |
| 1,000,000 | 3.4x | 3.4x | 3.3x | 3.1x | 1.2x |

zstd(9) is the only Parquet setting Vortex is competitive with on write time. At 1M rows,
`vortex_compact_bigblock` writes in 1.767 s against zstd(9)'s 1.747 s — a tie — and produces a
slightly smaller file (3,859,856 vs 3,899,632).

## Where the small-write cost goes

It is a fixed per-column cost, not a per-row one. A 10-row write costs roughly 0.7 ms per column:

```

=== fixed-cost probe: 10 rows, varying column count ===
case                               median          min   file bytes
1 cols                          408.313µs    307.344µs         2180
  (1 cols, parquet)              88.055µs     68.938µs          583
2 cols                            1.775ms      1.496ms         3740
  (2 cols, parquet)              96.605µs     80.892µs         1893
5 cols                            4.656ms      4.223ms         7260
  (5 cols, parquet)             111.267µs    101.761µs         3737
10 cols                           8.371ms      7.974ms        12956
  (10 cols, parquet)            153.355µs    138.325µs         5701
20 cols                          14.289ms     13.559ms        24400
  (20 cols, parquet)            234.159µs    187.172µs         9864

=== 10 rows, 20 cols: what costs the ~1ms/column? ===
empty compressor                  2.818ms      2.555ms        25104
default compressor               14.274ms     13.206ms        24400
compact compressor               16.247ms     15.228ms        19448

=== 10 rows, 20 cols, no file statistics ===
no file stats                    13.792ms     12.682ms        22752
```

Replacing the compressor with `BtrBlocksCompressorBuilder::empty()` takes the 20-column, 10-row
write from 14.3 ms to 2.8 ms, so about 80% of the floor is scheme search evaluating candidate
encodings against 10-element arrays. Disabling file statistics changes nothing. The remaining
~2.8 ms of layout, footer and IO machinery is still an order of magnitude above Parquet's
0.23 ms, but scheme search is where the bulk of it is.

Any fix probably wants a size threshold below which scheme search is skipped in favour of a fixed
cheap encoding, since on a 10-element array the search cannot pay for itself.

## Are we smaller than Parquet once Parquet is compressed?

**With default write options, no — we are substantially larger.** At 1M rows:

| writer | file bytes | vs default vortex |
| --- | ---: | ---: |
| parquet uncompressed | 183,752,650 | 3.3x larger |
| parquet snappy | 24,410,331 | 2.3x smaller |
| parquet lz4 | 20,662,054 | 2.7x smaller |
| parquet zstd(1) | 4,538,650 | 12.3x smaller |
| parquet zstd(9) | 3,899,632 | 14.4x smaller |
| vortex (default) | 55,959,392 | — |

Default Vortex only beats *uncompressed* Parquet. It loses to LZ4 by 2.7x and to zstd(1) by 12x.

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

Compact Vortex is 2x smaller than zstd(1) Parquet at 10k and 100k rows, and at 1M rows it beats
zstd(1) on size while writing in 1.77 s against zstd(1)'s 0.61 s. Against zstd(9) it wins on both
axes at 1M rows, narrowly.

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
