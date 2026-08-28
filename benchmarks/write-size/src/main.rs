// Reproduction of vortex-data/vortex#5861 plus a file-size comparison against
// *compressed* parquet.
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use arrow::array::{ArrayRef, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use tempfile::TempDir;
use vortex::dtype::DType;
use vortex::error::VortexError;
use vortex::compressor::BtrBlocksCompressorBuilder;
use vortex::file::{WriteOptionsSessionExt, WriteStrategyBuilder};
use vortex::layout::LayoutStrategy;
use vortex::io::runtime::BlockingRuntime;
use vortex::io::runtime::current::{CurrentThreadRuntime, CurrentThreadWorkerPool};
use vortex::io::session::RuntimeSessionExt;
use vortex::editions::{CORE_2026_08_3, EditionSessionExt};
use vortex::session::VortexSession;
use vortex::{VortexSessionDefault, array::ArrayRef as VortexArrayRef};
use vortex::arrow::{FromArrowArray, FromArrowType};
use vortex_array::iter::{ArrayIteratorAdapter, ArrayIteratorExt};

static RUNTIME: LazyLock<CurrentThreadRuntime> = LazyLock::new(CurrentThreadRuntime::new);
static SESSION: LazyLock<VortexSession> =
    LazyLock::new(|| VortexSession::default().with_handle(RUNTIME.handle()));
static TOKIO_RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
});
static TOKIO_SESSION: LazyLock<VortexSession> =
    LazyLock::new(|| VortexSession::default().with_tokio());

/// A second CurrentThreadRuntime driven by a background worker pool. Kept separate from
/// `RUNTIME` because the pool's workers drive the whole executor, so sharing one would
/// silently parallelise the single-threaded baseline too.
static POOL_RUNTIME: LazyLock<CurrentThreadRuntime> = LazyLock::new(CurrentThreadRuntime::new);
static POOL: LazyLock<CurrentThreadWorkerPool> = LazyLock::new(|| {
    let pool = POOL_RUNTIME.new_pool();
    pool.set_workers_to_available_parallelism();
    pool
});
static POOL_SESSION: LazyLock<VortexSession> =
    LazyLock::new(|| VortexSession::default().with_handle(POOL_RUNTIME.handle()));

static TOKIO_MT_RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
});
static TOKIO_MT_SESSION: LazyLock<VortexSession> =
    LazyLock::new(|| VortexSession::default().with_tokio());

/// Default session, but opted forward to the newest frozen core edition. The default is
/// CORE_2026_08_1; 08_2 adds Map arrays and 08_3 adds Variant arrays and the UUID dtype.
static LATEST_EDITION_SESSION: LazyLock<VortexSession> = LazyLock::new(|| {
    let session = VortexSession::default().with_handle(RUNTIME.handle());
    session
        .enable_edition(CORE_2026_08_3)
        .expect("CORE_2026_08_3 is registered");
    session
});

fn create_test_batch(num_rows: usize, batch_size: Option<usize>) -> Vec<RecordBatch> {
    let batch_size = batch_size.unwrap_or(8192);

    let schema = Arc::new(Schema::new(vec![
        Field::new("_timestamp", DataType::Int64, false),
        Field::new("log", DataType::Utf8, false),
        Field::new("kubernetes_namespace_name", DataType::Utf8, false),
        Field::new("kubernetes_container_name", DataType::Utf8, false),
        Field::new("url", DataType::Utf8, false),
        Field::new("host", DataType::Utf8, false),
        Field::new("pod_name", DataType::Utf8, false),
        Field::new("service_name", DataType::Utf8, false),
        Field::new("level", DataType::Utf8, false),
        Field::new("thread", DataType::Utf8, false),
        Field::new("request_id", DataType::Utf8, false),
        Field::new("user_id", DataType::Utf8, false),
        Field::new("session_id", DataType::Utf8, false),
        Field::new("method", DataType::Utf8, false),
        Field::new("path", DataType::Utf8, false),
        Field::new("status_code", DataType::Int64, false),
        Field::new("response_time_ms", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("environment", DataType::Utf8, false),
        Field::new("version", DataType::Utf8, false),
    ]));

    let num_batches = num_rows.div_ceil(batch_size);
    let mut batches = Vec::with_capacity(num_batches);

    for batch_idx in 0..num_batches {
        let start_idx = batch_idx * batch_size;
        let end_idx = std::cmp::min(start_idx + batch_size, num_rows);

        let timestamp_data: Vec<i64> = (start_idx..end_idx).map(|i| i as i64).collect();
        let log_data: Vec<String> = (start_idx..end_idx)
            .map(|i| {
                format!(
                    "This is log message number {} with some additional content to simulate real logs",
                    i
                )
            })
            .collect();
        let namespace_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("namespace-{}", i % 10))
            .collect();
        let container_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("container-{}", i % 20))
            .collect();
        let url_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("https://example.com/path/{}/resource?query={}", i % 100, i))
            .collect();
        let host_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("host-{}.example.com", i % 50))
            .collect();
        let pod_name_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("pod-{}-{}", i % 30, (i / 30) % 5))
            .collect();
        let service_name_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("service-{}", i % 15))
            .collect();
        let level_data: Vec<String> = (start_idx..end_idx)
            .map(|i| {
                match i % 5 {
                    0 => "ERROR",
                    1 => "WARN",
                    2 => "INFO",
                    3 => "DEBUG",
                    _ => "TRACE",
                }
                .to_string()
            })
            .collect();
        let thread_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("thread-{}", i % 8))
            .collect();
        let request_id_data: Vec<String> =
            (start_idx..end_idx).map(|i| format!("req-{:016x}", i)).collect();
        let user_id_data: Vec<String> =
            (start_idx..end_idx).map(|i| format!("user-{}", i % 1000)).collect();
        let session_id_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("session-{:016x}", i % 500))
            .collect();
        let method_data: Vec<String> = (start_idx..end_idx)
            .map(|i| {
                match i % 4 {
                    0 => "GET",
                    1 => "POST",
                    2 => "PUT",
                    _ => "DELETE",
                }
                .to_string()
            })
            .collect();
        let path_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("/api/v1/resource/{}/action", i % 200))
            .collect();
        let status_code_data: Vec<i64> = (start_idx..end_idx)
            .map(|i| match i % 10 {
                0 | 1 | 2 | 3 | 4 => 200,
                5 | 6 => 201,
                7 => 400,
                8 => 404,
                _ => 500,
            })
            .collect();
        let response_time_data: Vec<i64> =
            (start_idx..end_idx).map(|i| 10 + (i as i64 % 990)).collect();
        let region_data: Vec<String> = (start_idx..end_idx)
            .map(|i| {
                match i % 4 {
                    0 => "us-east-1",
                    1 => "us-west-2",
                    2 => "eu-west-1",
                    _ => "ap-southeast-1",
                }
                .to_string()
            })
            .collect();
        let environment_data: Vec<String> = (start_idx..end_idx)
            .map(|i| {
                match i % 3 {
                    0 => "production",
                    1 => "staging",
                    _ => "development",
                }
                .to_string()
            })
            .collect();
        let version_data: Vec<String> = (start_idx..end_idx)
            .map(|i| format!("v{}.{}.{}", (i % 3) + 1, (i % 10), (i % 5)))
            .collect();

        let arrays: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(timestamp_data)),
            Arc::new(StringArray::from(log_data)),
            Arc::new(StringArray::from(namespace_data)),
            Arc::new(StringArray::from(container_data)),
            Arc::new(StringArray::from(url_data)),
            Arc::new(StringArray::from(host_data)),
            Arc::new(StringArray::from(pod_name_data)),
            Arc::new(StringArray::from(service_name_data)),
            Arc::new(StringArray::from(level_data)),
            Arc::new(StringArray::from(thread_data)),
            Arc::new(StringArray::from(request_id_data)),
            Arc::new(StringArray::from(user_id_data)),
            Arc::new(StringArray::from(session_id_data)),
            Arc::new(StringArray::from(method_data)),
            Arc::new(StringArray::from(path_data)),
            Arc::new(Int64Array::from(status_code_data)),
            Arc::new(Int64Array::from(response_time_data)),
            Arc::new(StringArray::from(region_data)),
            Arc::new(StringArray::from(environment_data)),
            Arc::new(StringArray::from(version_data)),
        ];

        batches.push(RecordBatch::try_new(schema.clone(), arrays).unwrap());
    }

    batches
}

fn write_parquet(batches: &[RecordBatch], path: &Path, compression: Compression) {
    let props = WriterProperties::builder()
        .set_compression(compression)
        .build();
    let file = std::fs::File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, batches[0].schema(), Some(props)).unwrap();
    for batch in batches {
        writer.write(batch).unwrap();
    }
    writer.close().unwrap();
}

fn write_vortex_async(batches: &[RecordBatch], path: &Path) {
    TOKIO_RUNTIME.block_on(async {
        let schema = batches[0].schema();
        let batches_clone: Vec<RecordBatch> = batches.to_vec();

        let array_iter = ArrayIteratorAdapter::new(
            DType::from_arrow(schema),
            batches_clone
                .into_iter()
                .map(Ok::<RecordBatch, VortexError>)
                .map(|batch_result| batch_result.and_then(|b| VortexArrayRef::from_arrow(b, false))),
        );

        let mut f = tokio::fs::File::create(path).await.unwrap();

        TOKIO_SESSION
            .write_options()
            .write(&mut f, array_iter.into_array_stream())
            .await
            .unwrap();
    });
}

const ONE_MEG: u64 = 1 << 20;

/// Default write strategy, but with the BtrBlocks "compact" schemes (zstd for strings/binary,
/// pco for numerics) added. These are *not* on by default.
fn compact_strategy() -> Arc<dyn LayoutStrategy> {
    WriteStrategyBuilder::default()
        .with_btrblocks_builder(BtrBlocksCompressorBuilder::default().with_compact())
        .build()
}

/// Default schemes, but with much larger blocks handed to the compressor.
fn big_block_strategy() -> Arc<dyn LayoutStrategy> {
    WriteStrategyBuilder::default()
        .with_row_block_size(65_536)
        .with_data_block_target_bytes(Some(16 * ONE_MEG))
        .build()
}

/// Compact schemes *and* large blocks.
fn compact_big_block_strategy() -> Arc<dyn LayoutStrategy> {
    WriteStrategyBuilder::default()
        .with_btrblocks_builder(BtrBlocksCompressorBuilder::default().with_compact())
        .with_row_block_size(65_536)
        .with_data_block_target_bytes(Some(16 * ONE_MEG))
        .build()
}

fn write_vortex_blocking_with(
    batches: &[RecordBatch],
    path: &Path,
    strategy: Option<Arc<dyn LayoutStrategy>>,
) {
    let dtype = DType::from_arrow(batches[0].schema());
    let file = std::fs::File::create(path).unwrap();
    let mut options = SESSION.write_options();
    if let Some(strategy) = strategy {
        options = options.with_strategy(strategy);
    }
    let mut writer = options.blocking(&*RUNTIME).writer(file, dtype);
    for batch in batches {
        writer
            .push(VortexArrayRef::from_arrow(batch, false).unwrap())
            .unwrap();
    }
    writer.finish().unwrap();
}

fn write_vortex_blocking(batches: &[RecordBatch], path: &Path) {
    write_vortex_blocking_with(batches, path, None)
}

/// Same blocking writer, but with background workers driving the executor so the per-column
/// tasks the struct writer spawns can run concurrently.
fn write_vortex_pool_with(
    batches: &[RecordBatch],
    path: &Path,
    strategy: Option<Arc<dyn LayoutStrategy>>,
) {
    LazyLock::force(&POOL);
    let dtype = DType::from_arrow(batches[0].schema());
    let file = std::fs::File::create(path).unwrap();
    let mut options = POOL_SESSION.write_options();
    if let Some(strategy) = strategy {
        options = options.with_strategy(strategy);
    }
    let mut writer = options.blocking(&*POOL_RUNTIME).writer(file, dtype);
    for batch in batches {
        writer
            .push(VortexArrayRef::from_arrow(batch, false).unwrap())
            .unwrap();
    }
    writer.finish().unwrap();
}

fn write_vortex_pool(batches: &[RecordBatch], path: &Path) {
    write_vortex_pool_with(batches, path, None)
}

/// The async path on a multi-threaded Tokio runtime rather than a current-thread one.
fn write_vortex_async_mt(batches: &[RecordBatch], path: &Path) {
    TOKIO_MT_RUNTIME.block_on(async {
        let schema = batches[0].schema();
        let batches_clone: Vec<RecordBatch> = batches.to_vec();

        let array_iter = ArrayIteratorAdapter::new(
            DType::from_arrow(schema),
            batches_clone
                .into_iter()
                .map(Ok::<RecordBatch, VortexError>)
                .map(|batch_result| batch_result.and_then(|b| VortexArrayRef::from_arrow(b, false))),
        );

        let mut f = tokio::fs::File::create(path).await.unwrap();

        TOKIO_MT_SESSION
            .write_options()
            .write(&mut f, array_iter.into_array_stream())
            .await
            .unwrap();
    });
}

/// Default compressor and single-threaded runtime, but writing under the newest core edition.
fn write_vortex_latest_edition(batches: &[RecordBatch], path: &Path) {
    let dtype = DType::from_arrow(batches[0].schema());
    let file = std::fs::File::create(path).unwrap();
    let mut writer = LATEST_EDITION_SESSION
        .write_options()
        .blocking(&*RUNTIME)
        .writer(file, dtype);
    for batch in batches {
        writer
            .push(VortexArrayRef::from_arrow(batch, false).unwrap())
            .unwrap();
    }
    writer.finish().unwrap();
}

struct Measurement {
    name: &'static str,
    median: Duration,
    min: Duration,
    bytes: u64,
}

fn measure(
    name: &'static str,
    dir: &Path,
    ext: &str,
    batches: &[RecordBatch],
    f: impl Fn(&[RecordBatch], &Path),
) -> Measurement {
    let path = dir.join(format!("{name}.{ext}"));

    // Warmup (also pays any one-time session / codec init cost).
    f(batches, &path);

    let budget = Duration::from_secs(3);
    let max_iters = 100;
    let min_iters = 5;
    let mut times = Vec::new();
    let start = Instant::now();
    while times.len() < max_iters && (times.len() < min_iters || start.elapsed() < budget) {
        let t = Instant::now();
        f(batches, &path);
        times.push(t.elapsed());
    }
    times.sort();
    let bytes = std::fs::metadata(&path).unwrap().len();

    Measurement {
        name,
        median: times[times.len() / 2],
        min: times[0],
        bytes,
    }
}

/// Keep only the first `n` columns of each batch.
fn project(batches: &[RecordBatch], n: usize) -> Vec<RecordBatch> {
    batches
        .iter()
        .map(|b| b.project(&(0..n).collect::<Vec<_>>()).unwrap())
        .collect()
}

/// Where does the ~20ms floor on a 10-row Vortex write come from?
fn probe(dir: &Path) {
    println!("\n=== fixed-cost probe: 10 rows, varying column count ===");
    let batches = create_test_batch(10, None);
    println!("{:<28} {:>12} {:>12} {:>12}", "case", "median", "min", "file bytes");
    for ncols in [1usize, 2, 5, 10, 20] {
        let projected = project(&batches, ncols);
        let m = measure("vortex_blocking", dir, "vortex", &projected, write_vortex_blocking);
        println!(
            "{:<28} {:>12} {:>12} {:>12}",
            format!("{ncols} cols"),
            format!("{:.3?}", m.median),
            format!("{:.3?}", m.min),
            m.bytes
        );
        let m = measure("parquet", dir, "parquet", &projected, |b, p| {
            write_parquet(b, p, Compression::UNCOMPRESSED)
        });
        println!(
            "{:<28} {:>12} {:>12} {:>12}",
            format!("  ({ncols} cols, parquet)"),
            format!("{:.3?}", m.median),
            format!("{:.3?}", m.min),
            m.bytes
        );
    }

    println!("\n=== 10 rows, 20 cols: what costs the ~1ms/column? ===");
    let variants: Vec<(&str, Arc<dyn LayoutStrategy>)> = vec![
        (
            "empty compressor",
            WriteStrategyBuilder::default()
                .with_btrblocks_builder(BtrBlocksCompressorBuilder::empty())
                .build(),
        ),
        ("default compressor", WriteStrategyBuilder::default().build()),
        (
            "compact compressor",
            WriteStrategyBuilder::default()
                .with_btrblocks_builder(BtrBlocksCompressorBuilder::default().with_compact())
                .build(),
        ),
    ];
    for (name, strategy) in variants {
        let m = measure("v", dir, "vortex", &batches, move |b, p| {
            write_vortex_blocking_with(b, p, Some(strategy.clone()))
        });
        println!(
            "{:<28} {:>12} {:>12} {:>12}",
            name,
            format!("{:.3?}", m.median),
            format!("{:.3?}", m.min),
            m.bytes
        );
    }

    println!("\n=== 10 rows, 20 cols, no file statistics ===");
    let m = measure("vortex_nostats", dir, "vortex", &batches, |b, p| {
        let dtype = DType::from_arrow(b[0].schema());
        let file = std::fs::File::create(p).unwrap();
        let mut writer = SESSION
            .write_options()
            .with_file_statistics(vec![])
            .blocking(&*RUNTIME)
            .writer(file, dtype);
        for batch in b {
            writer
                .push(VortexArrayRef::from_arrow(batch, false).unwrap())
                .unwrap();
        }
        writer.finish().unwrap();
    });
    println!("{:<28} {:>12} {:>12} {:>12}", "no file stats", format!("{:.3?}", m.median), format!("{:.3?}", m.min), m.bytes);
}

/// Sample the single-threaded blocking write and attribute the samples to compressor internals.
#[cfg(target_os = "linux")]
fn profile(dir: &Path, rows: usize, seconds: u64) {
    use std::collections::HashMap;

    let batches = create_test_batch(rows, None);
    let path = dir.join("profile.vortex");

    // Warm up so one-time init does not land in the profile.
    write_vortex_blocking(&batches, &path);

    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(999)
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut iters = 0u64;
    while Instant::now() < deadline {
        write_vortex_blocking(&batches, &path);
        iters += 1;
    }

    let report = guard.report().build().unwrap();
    println!("\n=== profile: {rows} rows, vortex_blocking, {iters} iterations ===");

    // Attribution walks from the leaf outward and takes the FIRST frame that matches a bucket.
    // Matching anywhere in the stack would credit everything to whichever scheme sits highest,
    // which is wrong for a cascading compressor: FSST's integer codes are then bitpacked, so
    // the bitpacking frames have FSST ancestors.
    let buckets: &[(&str, &[&str])] = &[
        ("fsst (string fragmentation)", &["fsst"]),
        ("onpair (string fragmentation)", &["onpair"]),
        ("zstd", &["zstd"]),
        ("pco", &["pco"]),
        ("dict build/probe", &["dict"]),
        ("bitpacking / FoR / zigzag", &["fastlanes", "bitpack", "zigzag"]),
        ("runend / RLE / sequence", &["runend", "sequence"]),
        ("alp", &["alp"]),
        ("compressor stats (scheme scoring)", &["vortex_compressor::stats"]),
        ("zone map stats / aggregates", &["aggregate_fn", "min_max", "zone"]),
        ("varbinview construction", &["varbinview", "binaryview"]),
        ("scheme search / cascade", &["btrblocks", "cascad", "scheme", "sample"]),
        ("arrow conversion", &["arrow"]),
        ("layout / segments / footer", &["layout", "segment", "footer", "flatbuffer"]),
        ("hashing", &["hashbrown", "hash"]),
        ("alloc / memcpy", &["alloc", "malloc", "memcpy", "__rust_"]),
    ];

    let mut by_bucket: HashMap<&str, usize> = HashMap::new();
    let mut by_leaf: HashMap<String, usize> = HashMap::new();
    let mut by_crate: HashMap<String, usize> = HashMap::new();
    let mut total = 0usize;

    for (frames, count) in report.data.iter() {
        let count = *count as usize;
        total += count;

        // pprof orders frames leaf-first.
        let names: Vec<String> = frames
            .frames
            .iter()
            .flat_map(|f| f.iter())
            .map(|sym| format!("{sym}"))
            .collect();

        if let Some(leaf) = names.first() {
            *by_leaf.entry(leaf.clone()).or_default() += count;
            let krate = leaf
                .trim_start_matches('<')
                .split("::")
                .next()
                .unwrap_or("?")
                .split(&[' ', '<'][..])
                .next()
                .unwrap_or("?")
                .to_string();
            *by_crate.entry(krate).or_default() += count;
        }

        let bucket = names
            .iter()
            .find_map(|name| {
                let lower = name.to_lowercase();
                buckets
                    .iter()
                    .find(|(_, needles)| needles.iter().any(|needle| lower.contains(needle)))
                    .map(|(label, _)| *label)
            })
            .unwrap_or("other");
        *by_bucket.entry(bucket).or_default() += count;
    }

    let pct = |c: usize| 100.0 * c as f64 / total as f64;

    println!("\n-- samples by nearest-frame area ({total} samples total) --");
    let mut rows_out: Vec<_> = by_bucket.into_iter().collect();
    rows_out.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (name, count) in rows_out {
        println!("{:<38} {:>7} {:>7.1}%", name, count, pct(count));
    }

    println!("\n-- self time by crate --");
    let mut crates: Vec<_> = by_crate.into_iter().collect();
    crates.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (name, count) in crates.into_iter().take(15) {
        println!("{:<38} {:>7} {:>7.1}%", name, count, pct(count));
    }

    println!("\n-- top 25 leaf functions (self time) --");
    let mut leaves: Vec<_> = by_leaf.into_iter().collect();
    leaves.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (name, count) in leaves.into_iter().take(25) {
        let short: String = name.chars().take(110).collect();
        println!("{:>7} {:>6.2}%  {}", count, pct(count), short);
    }

    let flamegraph_path = std::path::Path::new("flamegraph.svg");
    let file = std::fs::File::create(flamegraph_path).unwrap();
    report.flamegraph(file).unwrap();
    println!("\nflamegraph written to {}", flamegraph_path.display());
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("profile") {
        #[cfg(target_os = "linux")]
        {
            let dir = TempDir::new().unwrap();
            let rows: usize = std::env::args()
                .nth(2)
                .and_then(|a| a.parse().ok())
                .unwrap_or(100_000);
            let seconds: u64 = std::env::args()
                .nth(3)
                .and_then(|a| a.parse().ok())
                .unwrap_or(20);
            profile(dir.path(), rows, seconds);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("probe") {
        let dir = TempDir::new().unwrap();
        probe(dir.path());
        return;
    }
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let sizes = if sizes.is_empty() {
        vec![10, 1_000, 10_000, 100_000, 1_000_000]
    } else {
        sizes
    };

    let dir = TempDir::new().unwrap();

    for size in sizes {
        let batches = create_test_batch(size, None);
        let arrow_bytes: usize = batches.iter().map(|b| b.get_array_memory_size()).sum();

        println!(
            "\n=== {size} rows, {} batch(es), {} arrow in-memory bytes ===",
            batches.len(),
            arrow_bytes
        );

        let results = vec![
            measure("parquet_uncompressed", dir.path(), "parquet", &batches, |b, p| {
                write_parquet(b, p, Compression::UNCOMPRESSED)
            }),
            measure("parquet_snappy", dir.path(), "parquet", &batches, |b, p| {
                write_parquet(b, p, Compression::SNAPPY)
            }),
            measure("parquet_lz4", dir.path(), "parquet", &batches, |b, p| {
                write_parquet(b, p, Compression::LZ4_RAW)
            }),
            measure("parquet_lz4_hadoop", dir.path(), "parquet", &batches, |b, p| {
                write_parquet(b, p, Compression::LZ4)
            }),
            measure("parquet_zstd1", dir.path(), "parquet", &batches, |b, p| {
                write_parquet(b, p, Compression::ZSTD(ZstdLevel::try_new(1).unwrap()))
            }),
            measure("parquet_zstd3", dir.path(), "parquet", &batches, |b, p| {
                write_parquet(b, p, Compression::ZSTD(ZstdLevel::try_new(3).unwrap()))
            }),
            measure("parquet_zstd9", dir.path(), "parquet", &batches, |b, p| {
                write_parquet(b, p, Compression::ZSTD(ZstdLevel::try_new(9).unwrap()))
            }),
            measure("vortex_async", dir.path(), "vortex", &batches, write_vortex_async),
            measure("vortex_blocking", dir.path(), "vortex", &batches, write_vortex_blocking),
            measure("vortex_compact", dir.path(), "vortex", &batches, |b, p| {
                write_vortex_blocking_with(b, p, Some(compact_strategy()))
            }),
            measure("vortex_bigblock", dir.path(), "vortex", &batches, |b, p| {
                write_vortex_blocking_with(b, p, Some(big_block_strategy()))
            }),
            measure("vortex_compact_bigblock", dir.path(), "vortex", &batches, |b, p| {
                write_vortex_blocking_with(b, p, Some(compact_big_block_strategy()))
            }),
            measure("vortex_pool", dir.path(), "vortex", &batches, write_vortex_pool),
            measure("vortex_pool_compact", dir.path(), "vortex", &batches, |b, p| {
                write_vortex_pool_with(b, p, Some(compact_strategy()))
            }),
            measure("vortex_async_mt", dir.path(), "vortex", &batches, write_vortex_async_mt),
            measure("vortex_latest_edition", dir.path(), "vortex", &batches, write_vortex_latest_edition),
        ];

        println!(
            "{:<22} {:>12} {:>12} {:>14} {:>12} {:>10}",
            "writer", "median", "min", "file bytes", "MiB/s", "vs arrow"
        );
        for m in &results {
            let secs = m.median.as_secs_f64();
            let thrpt = (arrow_bytes as f64 / (1024.0 * 1024.0)) / secs;
            println!(
                "{:<22} {:>12} {:>12} {:>14} {:>12.1} {:>9.2}x",
                m.name,
                format!("{:.3?}", m.median),
                format!("{:.3?}", m.min),
                m.bytes,
                thrpt,
                arrow_bytes as f64 / m.bytes as f64,
            );
        }
    }
}
