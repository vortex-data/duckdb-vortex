// Reproduction of vortex-data/vortex#5861 plus a file-size comparison against
// *compressed* parquet.
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use arrow::array::{ArrayRef, Int64Array, StringArray, StringViewArray};
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

/// Which Arrow string representation the generator emits. Vortex's canonical string array is
/// VarBinView, and its Arrow export for `DType::Utf8` is `Utf8View`, so `Utf8` input has to be
/// converted on the way in while `Utf8View` should not.
#[derive(Copy, Clone, PartialEq, Eq)]
enum StringKind {
    Utf8,
    Utf8View,
}

impl StringKind {
    fn data_type(self) -> DataType {
        match self {
            StringKind::Utf8 => DataType::Utf8,
            StringKind::Utf8View => DataType::Utf8View,
        }
    }

    fn array(self, values: Vec<String>) -> ArrayRef {
        match self {
            StringKind::Utf8 => Arc::new(StringArray::from(values)),
            StringKind::Utf8View => Arc::new(StringViewArray::from_iter_values(values)),
        }
    }
}

fn create_test_batch(num_rows: usize, batch_size: Option<usize>) -> Vec<RecordBatch> {
    create_test_batch_of(num_rows, batch_size, StringKind::Utf8View)
}

fn create_test_batch_of(
    num_rows: usize,
    batch_size: Option<usize>,
    kind: StringKind,
) -> Vec<RecordBatch> {
    let batch_size = batch_size.unwrap_or(8192);

    let schema = Arc::new(Schema::new(vec![
        Field::new("_timestamp", DataType::Int64, false),
        Field::new("log", kind.data_type(), false),
        Field::new("kubernetes_namespace_name", kind.data_type(), false),
        Field::new("kubernetes_container_name", kind.data_type(), false),
        Field::new("url", kind.data_type(), false),
        Field::new("host", kind.data_type(), false),
        Field::new("pod_name", kind.data_type(), false),
        Field::new("service_name", kind.data_type(), false),
        Field::new("level", kind.data_type(), false),
        Field::new("thread", kind.data_type(), false),
        Field::new("request_id", kind.data_type(), false),
        Field::new("user_id", kind.data_type(), false),
        Field::new("session_id", kind.data_type(), false),
        Field::new("method", kind.data_type(), false),
        Field::new("path", kind.data_type(), false),
        Field::new("status_code", DataType::Int64, false),
        Field::new("response_time_ms", DataType::Int64, false),
        Field::new("region", kind.data_type(), false),
        Field::new("environment", kind.data_type(), false),
        Field::new("version", kind.data_type(), false),
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
            kind.array(log_data),
            kind.array(namespace_data),
            kind.array(container_data),
            kind.array(url_data),
            kind.array(host_data),
            kind.array(pod_name_data),
            kind.array(service_name_data),
            kind.array(level_data),
            kind.array(thread_data),
            kind.array(request_id_data),
            kind.array(user_id_data),
            kind.array(session_id_data),
            kind.array(method_data),
            kind.array(path_data),
            Arc::new(Int64Array::from(status_code_data)),
            Arc::new(Int64Array::from(response_time_data)),
            kind.array(region_data),
            kind.array(environment_data),
            kind.array(version_data),
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

    // Attribution walks leaf-outward to the first frame belonging to a crate we are attributing
    // work to, and buckets on that frame's *defining path* with generic parameters stripped.
    //
    // Both halves matter. Matching raw symbols credits a frame by its type parameters, so the
    // min/max scan's `Iter<BinaryView>` reads as VarBinView work; stripping generics fixes that.
    // Skipping non-domain frames pushes std/arrow-buffer plumbing (memcpy, BitIterator, hashbrown
    // probes) onto the vortex function that invoked it, which is what "where does the time go"
    // actually means.
    fn base_path(sym: &str) -> String {
        let s = sym.trim();
        if let Some(rest) = s.strip_prefix('<') {
            // `<Type as Trait>::method` — attribute to Type, not to the trait.
            let mut depth = 1usize;
            let mut end = rest.len();
            for (i, c) in rest.char_indices() {
                match c {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let inner = &rest[..end];
            return base_path(inner.split(" as ").next().unwrap_or(inner));
        }
        let mut out = String::with_capacity(s.len());
        let mut depth = 0usize;
        for c in s.chars() {
            match c {
                '<' => depth += 1,
                '>' => depth = depth.saturating_sub(1),
                _ if depth == 0 => out.push(c),
                _ => {}
            }
        }
        out
    }

    // arrow_buffer is deliberately absent: its BitIterator is plumbing for whoever iterates a
    // validity mask, not conversion work of its own.
    fn is_domain(base: &str) -> bool {
        ["vortex", "fsst", "pco", "zstd"]
            .iter()
            .any(|d| base.starts_with(d))
    }

    let buckets: &[(&str, &[&str])] = &[
        ("fsst", &["fsst"]),
        ("onpair", &["onpair"]),
        ("zstd", &["zstd"]),
        ("pco", &["pco"]),
        ("dict builder (builds VarBinView)", &["builders::dict"]),
        ("dict encoding array", &["arrays::dict"]),
        ("zone map stats / min-max", &["aggregate_fn", "min_max", "stats::"]),
        ("compressor stats (scheme scoring)", &["vortex_compressor"]),
        ("bitpacking / FoR / zigzag", &["fastlanes", "zigzag"]),
        ("runend / sequence", &["runend", "sequence"]),
        ("alp", &["alp"]),
        ("varbinview array ops", &["varbinview"]),
        ("canonicalize", &["canonical"]),
        ("scheme search / cascade", &["btrblocks", "cascad", "scheme", "sample"]),
        ("arrow conversion", &["vortex_arrow"]),
        ("layout / segments / footer", &["layout", "segment", "footer", "flatbuffer"]),
        ("buffer / builders", &["buffer", "builders"]),
    ];

    let mut by_bucket: HashMap<&str, usize> = HashMap::new();
    let mut by_fn: HashMap<String, usize> = HashMap::new();
    let mut by_leaf_crate: HashMap<String, usize> = HashMap::new();
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
            let krate = base_path(leaf)
                .split("::")
                .next()
                .unwrap_or("?")
                .to_string();
            *by_leaf_crate.entry(krate).or_default() += count;
        }

        let attributed = names.iter().map(|n| base_path(n)).find(|b| is_domain(b));

        match attributed {
            Some(base) => {
                *by_fn.entry(base.clone()).or_default() += count;
                let lower = base.to_lowercase();
                let bucket = buckets
                    .iter()
                    .find(|(_, needles)| needles.iter().any(|n| lower.contains(n)))
                    .map(|(label, _)| *label)
                    .unwrap_or("other vortex");
                *by_bucket.entry(bucket).or_default() += count;
            }
            None => *by_bucket.entry("runtime / std only").or_default() += count,
        }
    }

    let pct = |c: usize| 100.0 * c as f64 / total as f64;

    println!("\n-- samples by area ({total} samples total) --");
    let mut rows_out: Vec<_> = by_bucket.into_iter().collect();
    rows_out.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (name, count) in rows_out {
        println!("{:<40} {:>7} {:>7.1}%", name, count, pct(count));
    }

    println!("\n-- leaf frame's own crate (who burns the cycles) --");
    let mut crates: Vec<_> = by_leaf_crate.into_iter().collect();
    crates.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (name, count) in crates.into_iter().take(12) {
        println!("{:<40} {:>7} {:>7.1}%", name, count, pct(count));
    }

    println!("\n-- top 20 attributed functions --");
    let mut fns: Vec<_> = by_fn.into_iter().collect();
    fns.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (name, count) in fns.into_iter().take(20) {
        let short: String = name.chars().take(100).collect();
        println!("{:>7} {:>6.2}%  {}", count, pct(count), short);
    }

    let flamegraph_path = std::path::Path::new("flamegraph.svg");
    let file = std::fs::File::create(flamegraph_path).unwrap();
    report.flamegraph(file).unwrap();
    println!("\nflamegraph written to {}", flamegraph_path.display());
}

/// Measure the Vortex writers against both Arrow string representations back to back, in one
/// process, so the comparison is not confounded by run-to-run drift on the host.
fn viewcmp(dir: &Path, sizes: &[usize]) {
    for &size in sizes {
        let utf8 = create_test_batch_of(size, None, StringKind::Utf8);
        let view = create_test_batch_of(size, None, StringKind::Utf8View);
        let utf8_bytes: usize = utf8.iter().map(|b| b.get_array_memory_size()).sum();
        let view_bytes: usize = view.iter().map(|b| b.get_array_memory_size()).sum();

        println!(
            "\n=== {size} rows: Utf8 vs Utf8View input (arrow in-memory {utf8_bytes} vs {view_bytes} bytes) ==="
        );
        println!(
            "{:<24} {:>12} {:>12} {:>9} {:>12} {:>12}",
            "writer", "utf8", "utf8view", "change", "utf8 bytes", "view bytes"
        );

        type Writer = (&'static str, fn(&[RecordBatch], &Path));
        let writers: &[Writer] = &[
            ("vortex_blocking", write_vortex_blocking),
            ("vortex_pool", write_vortex_pool),
            ("vortex_async", write_vortex_async),
            ("parquet_zstd1", |b, p| {
                write_parquet(b, p, Compression::ZSTD(ZstdLevel::try_new(1).unwrap()))
            }),
        ];

        for (name, f) in writers {
            let a = measure(name, dir, "out", &utf8, f);
            let b = measure(name, dir, "out", &view, f);
            let change = 100.0 * (b.median.as_secs_f64() / a.median.as_secs_f64() - 1.0);
            println!(
                "{:<24} {:>12} {:>12} {:>+8.1}% {:>12} {:>12}",
                name,
                format!("{:.3?}", a.median),
                format!("{:.3?}", b.median),
                change,
                a.bytes,
                b.bytes
            );
        }

        // Compact too, since it is the configuration worth shipping.
        let a = measure("compact", dir, "out", &utf8, |b, p| {
            write_vortex_pool_with(b, p, Some(compact_strategy()))
        });
        let b = measure("compact", dir, "out", &view, |b, p| {
            write_vortex_pool_with(b, p, Some(compact_strategy()))
        });
        let change = 100.0 * (b.median.as_secs_f64() / a.median.as_secs_f64() - 1.0);
        println!(
            "{:<24} {:>12} {:>12} {:>+8.1}% {:>12} {:>12}",
            "vortex_pool_compact",
            format!("{:.3?}", a.median),
            format!("{:.3?}", b.median),
            change,
            a.bytes,
            b.bytes
        );
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("viewcmp") {
        let dir = TempDir::new().unwrap();
        let sizes: Vec<usize> = std::env::args()
            .skip(2)
            .map(|a| a.parse().unwrap())
            .collect();
        let sizes = if sizes.is_empty() {
            vec![1_000, 100_000, 1_000_000]
        } else {
            sizes
        };
        viewcmp(dir.path(), &sizes);
        return;
    }
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
