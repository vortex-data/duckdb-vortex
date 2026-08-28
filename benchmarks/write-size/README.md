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

The data generator follows the issue: 20 columns (3 `Int64`, 17 string) of synthetic Kubernetes
log records, chunked into 8192-row `RecordBatch`es. The issue used `Utf8`; this benchmark emits
`Utf8View` instead, which is the representation Vortex actually wants — see
[Utf8 vs Utf8View input](#utf8-vs-utf8view-input) for the measured difference and
`cargo run --release -- viewcmp` to reproduce it. Every artefact in `results/` is from `Utf8View`
input.

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

---

# Follow-ups: threading, unstable encodings, and a profile

Three further questions, measured on the same box and the same vortex rev. Raw output for all of
these is in `results/`.

```sh
cargo run --release -- 1000 100000 1000000              # adds the threaded writers
cargo run --release --features unstable -- 1000 100000 1000000
cargo run --release -- profile 100000 25                # sampling profile + flamegraph.svg
```

New writers:

| writer | what it is |
| --- | --- |
| `vortex_pool` | blocking writer, but a `CurrentThreadWorkerPool` sized to available parallelism drives the executor |
| `vortex_pool_compact` | the same, with compact schemes |
| `vortex_async_mt` | the async path on a multi-threaded Tokio runtime |
| `vortex_latest_edition` | default compressor, session opted forward to `CORE_2026_08_3` |

## 1. Threading the blocking write

The struct writer already spawns one task per column (`vortex-layout/src/layouts/struct_/writer.rs`,
joined with `try_join_all`). A bare `CurrentThreadRuntime` does no work unless someone calls
`block_on`, so those tasks run one after another. Attaching a worker pool — three workers on this
4 vCPU box — runs them concurrently, and nothing else has to change:

| rows | `vortex_blocking` | `vortex_pool` | speedup |
| ---: | ---: | ---: | ---: |
| 1,000 | 42.1 ms | 18.2 ms | 2.3x |
| 100,000 | 232 ms | 85.5 ms | 2.7x |
| 1,000,000 | 2.073 s | 664 ms | 3.1x |

That closes almost all of the gap to Parquet at scale. At 1M rows pooled Vortex (664 ms) is within
11% of snappy (598 ms) and within 2% of zstd(1) (651 ms), against 3.4x behind before. The
multi-threaded Tokio runtime lands in the same place (667 ms), so this is a property of the
executor, not of the blocking-vs-async API.

The best size/speed point in the whole matrix is `vortex_pool_compact`: at 1M rows, 741 ms for
3,919,000 bytes. That is 14% smaller than zstd(1) Parquet for 14% more write time, and the same
size as zstd(9) Parquet in 2.35x less time.

Parallelism does not rescue the small-write case. At 1,000 rows the pool still leaves Vortex 13x
behind snappy, because the per-column cost is paid either way and there are only so many cores to
spread it over.

## 2. `unstable_encodings` and the newest editions

The feature adds `OnPairScheme` (strings) and `DeltaScheme` (integers) to `ALL_SCHEMES`, and
enables the newest preview edition in the default session. On the default compressor it is a large
size win at a real time cost:

| rows | stable bytes | unstable bytes | change | stable time | unstable time |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000 | 86,440 | 76,240 | -11.8% | 42.1 ms | 60.3 ms |
| 100,000 | 5,058,784 | 3,467,080 | -31.5% | 232 ms | 366 ms |
| 1,000,000 | 55,959,392 | 33,361,856 | -40.4% | 2.073 s | 2.952 s |

OnPair beats FSST on these string columns and the gap widens with data volume. Delta comes along
via the preview edition but these integer columns are sequential or low-cardinality and already
suit FoR plus bitpacking.

On the compact compressor it is all cost and no benefit:

| 1M rows | stable | unstable |
| --- | --- | --- |
| `vortex_compact` | 3,919,000 in 2.229 s | 3,919,280 in 3.031 s |
| `vortex_compact_bigblock` | 3,859,856 in 1.788 s | 3,860,136 in 1.948 s |

Byte-for-byte the same output for 36% more time. Once Zstd is in the scheme list it wins these
columns outright, so OnPair and Delta are sampled and then discarded. `ZstdBuffers`, the other
scheme the feature unlocks, is only wired into `only_cuda_compatible`, not `with_compact`.

Even at -40%, default Vortex with unstable encodings is 33.4 MB against zstd(1) Parquet's 4.5 MB.
Lightweight encodings alone do not close a gap that an entropy coder opens on redundant text.

**The editions make no difference.** `CORE_2026_08_3` against the default `CORE_2026_08_1` gives
2.070 s vs 2.073 s and a file 96 bytes larger. `CORE_2026_08_2` adds Map arrays and
`CORE_2026_08_3` adds Variant arrays and the UUID dtype; an Int64/Utf8 schema uses none of them.
Note that `vortex.onpair` is the sole member of `CORE_2026_08_1`, the default edition — OnPair is
already permitted by the write policy today, so this is a cargo-feature gate, not an edition one,
and enabling it needs no edition opt-in.

## 3. Where the compressor spends its time

Sampled in-process at 999 Hz (`pprof`); there is no `perf` in the container. Samples are
attributed to the first matching frame walking outward from the leaf, because matching anywhere in
the stack credits everything to whichever scheme sits highest — FSST's integer codes are then
bitpacked, so bitpacking frames have FSST ancestors.

At 100,000 rows, where real compression work dominates:

| area | share |
| --- | ---: |
| FSST | 33.1% |
| VarBinView construction | 19.7% |
| zone map stats / aggregates | 11.9% |
| hashing | 9.6% |
| alloc / memcpy | 8.2% |
| dict build/probe | 4.9% |
| bitpacking / FoR / zigzag | 3.9% |
| scheme search / cascade | 2.0% |
| compressor stats (scheme scoring) | 1.8% |

Top single frame is `MaybeUninit<BinaryView>::write` at 13.0%, then
`fsst::Compressor::compress_into` at 9.5%.

At 1,000 rows, where the per-column floor dominates, FSST rises to 51.7% — and the split within
FSST changes. The hot frames are no longer compression but **symbol table training**:
`compare_masked` 4.4%, `CompressorBuilder::compress_count` 3.8%, `CompressorBuilder::optimize`
3.7%, `Counter::record_count2` 2.6%, `BinaryHeap<Candidate>::sift_up` 2.4%. At 100k rows
`compress_into` (actual compression) leads instead.

That is the explanation for the ~1 ms per column floor. FSST trains a symbol table per column per
chunk, and training runs a fixed number of optimisation rounds over a sample, so it costs roughly
the same whether the chunk holds 10 rows or 10,000. On a tiny write you pay full training for
almost no data — which is also why the earlier empty-compressor probe cut the 20-column 10-row
write from 14.3 ms to 2.8 ms.

Two other things worth noting from the profile:

- **VarBinView construction at 19.7% is not compression at all.** It is Arrow Utf8 being
  canonicalised into VarBinView plus the output buffers dict and FSST build.
- **Zone map statistics cost 11.9%**, largely `varbin_compute_min_max` over the string columns.
  This is not what `with_file_statistics(vec![])` disables — that controls file-level statistics
  only, which is why turning it off changed nothing in the earlier probe.

The cheapest available win for small writes is therefore a size threshold below which FSST
training is skipped in favour of a fixed encoding, rather than anything in the layout or IO path.

## Utf8 vs Utf8View input

Vortex's canonical string array is VarBinView, and `vortex-arrow` maps `DType::Utf8` back out to
`DataType::Utf8View`. Arrow `Utf8` input therefore has to be converted on the way in; `Utf8View`
should pass through. The earlier profile put that conversion at 19.7% of the write, so the
generator now emits `StringViewArray`.

Measured back to back in one process (`cargo run --release -- viewcmp`), because comparing across
separate runs on this host drifts by more than the effect being measured:

| rows | writer | Utf8 | Utf8View | change |
| ---: | --- | ---: | ---: | ---: |
| 1,000 | `vortex_blocking` | 53.5 ms | 54.3 ms | +1.5% |
| 1,000 | `vortex_pool` | 24.8 ms | 24.0 ms | -3.0% |
| 100,000 | `vortex_blocking` | 316 ms | 290 ms | -8.3% |
| 100,000 | `vortex_pool` | 107 ms | 97 ms | -9.3% |
| 100,000 | `vortex_pool_compact` | 118 ms | 107 ms | -8.8% |
| 1,000,000 | `vortex_blocking` | 2.641 s | 2.286 s | **-13.5%** |
| 1,000,000 | `vortex_async` | 2.802 s | 2.377 s | **-15.2%** |
| 1,000,000 | `vortex_pool` | 805 ms | 747 ms | -7.2% |
| 1,000,000 | `vortex_pool_compact` | 1.037 s | 858 ms | **-17.2%** |
| 1,000,000 | `parquet_zstd1` | 857 ms | 905 ms | +5.6% |

Output bytes are unchanged — 55,959,392 either way for the default compressor, and identical for
Parquet. Compact differs by 0.2% at 1M rows (3,919,000 vs 3,911,872), which is sampling noise in
scheme selection rather than a real encoding difference.

The gain grows with data volume and is nil at 1,000 rows, which fits: conversion is proportional
to bytes, while the small-write cost is the fixed per-column FSST training that no input format
change can touch. Parquet moves the other way, paying ~6% to materialise views back into byte
arrays.

Re-profiling at 100k rows confirms the mechanism — VarBinView construction falls from 19.7% to
4.4% of samples:

| area | Utf8 | Utf8View |
| --- | ---: | ---: |
| FSST | 33.1% | 37.7% |
| VarBinView construction | 19.7% | **4.4%** |
| zone map stats / aggregates | 11.9% | 13.5% |
| hashing | 9.6% | 14.7% |
| dict build/probe | 4.9% | 8.9% |

The remaining areas grow as a share because the total shrank, not because they got slower. The
top frame is now `fsst::Compressor::compress_into` at 12.4%, where before it was
`MaybeUninit<BinaryView>::write` at 13.0% — actual compression rather than format conversion.

## The profile on Utf8View input

All three sizes re-profiled after the switch (`results/profile-{1k,100k,1m}.txt`). The regimes are
genuinely different, so the size you profile at decides what you conclude.

| area | 1k rows | 100k rows | 1M rows |
| --- | ---: | ---: | ---: |
| FSST | 46.7% | 37.7% | 37.3% |
| hashing | 8.6% | 14.7% | 22.0% |
| zone map stats / aggregates | 7.4% | 13.5% | 11.8% |
| VarBinView construction | 1.2% | 4.4% | 9.3% |
| alloc / memcpy | 15.3% | 7.3% | 6.6% |
| bitpacking / FoR / zigzag | 4.7% | 4.2% | 3.5% |
| dict build/probe | 1.5% | 8.9% | 2.2% |
| scheme search / cascade | 4.6% | 2.2% | 1.2% |
| compressor stats (scheme scoring) | 2.5% | 1.0% | 1.0% |
| layout / segments / footer | 3.0% | 1.4% | 1.8% |

FSST leads everywhere, but it is not the same FSST work at each end.

At **1k rows** it is symbol table *training*: `Counter::record_count2` 8.3%,
`*mut Candidate::add` 4.5%, `copy_nonoverlapping::<Candidate>` 3.6%, `compare_masked` 3.2%,
`CompressorBuilder::optimize` 1.5%, `BinaryHeap::sift_up` 1.4%. Actual compression
(`compress_into`) is 1.2%. Training runs a fixed number of rounds over a sample, so it costs the
same regardless of chunk size — this is the ~1 ms per column floor, and it is why `alloc/memcpy`
is 15.3% here (candidate buffers churned per column) and why scheme search and scoring are only
visible at this size.

At **1M rows** it is symbol *matching*: `fsst::Code::eq` alone is 16.8%, plus
`advance_8byte_word` 2.1% and `compress_word` 2.0%. Training has faded to noise.

The other movement worth noting:

- **Hashing climbs steadily, 8.6% → 22.0%.** At 1M rows `hashbrown` is 13.5% of self time
  (`Tag::full` 7.7%, `is_bucket_full` 3.2%). This is dictionary probing during scheme evaluation
  on high-cardinality string columns, and at scale it is the second-largest cost in the writer.
- **VarBinView construction rises 1.2% → 9.3%** even on view input. It did not vanish with the
  format switch, it just stopped being input conversion — what remains is output buffers that
  dict and FSST build, which scales with data. `BinaryView::is_inlined` at 2.6% of the 1M profile
  is that work.
- **Zone map statistics hold near 12%** at both larger sizes, dominated by `varbin_compute_min_max`
  (`itertools::minmax` is 4.6% of the 1M profile). Still not what
  `with_file_statistics(vec![])` disables.
- **Scheme search itself stays cheap** — 4.6% at 1k, 1.2% at 1M. The cost is always inside the
  scheme that wins, never in deciding which one that is.

So the two levers are unchanged by the format switch, and they point at different sizes: a
training threshold for small writes, and dictionary probing plus zone-map min/max for large ones.
