use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use pprof::criterion::PProfProfiler;
use rkyv::{util::AlignedVec, Archive, Deserialize, Serialize, rancor::Error as RkyvErr};
#[cfg(unix)]
use wyhash::WyHash;

#[cfg(unix)]
use mmap_sync::locks::{LockDisabled, SingleWriter};
use mmap_sync::synchronizer::Synchronizer;
/// Example data-structure shared between writer and reader(s)
#[derive(Archive, Deserialize, Serialize, Debug, PartialEq)]
pub struct HelloWorld {
    pub version: u32,
    pub messages: Vec<String>,
}

fn build_mock_data() -> (HelloWorld, AlignedVec) {
    let data = HelloWorld {
        version: 7,
        messages: vec!["Hello".to_string(), "World".to_string(), "!".to_string()],
    };
    let bytes = rkyv::to_bytes::<RkyvErr>(&data).unwrap();

    (data, bytes)
}

pub fn bench_synchronizer(c: &mut Criterion) {
    let mut synchronizer = Synchronizer::new("/dev/shm/hello_world".as_ref());
    let (data, bytes) = build_mock_data();

    let mut group = c.benchmark_group("synchronizer");
    group.throughput(Throughput::Elements(1));

    group.bench_function("write", |b| {
        b.iter(|| {
            synchronizer
                .write(black_box(&data), Duration::from_nanos(10))
                .expect("failed to write data");
        })
    });

    group.bench_function("write_raw", |b| {
        b.iter(|| {
            synchronizer
                .write_raw::<HelloWorld>(black_box(&bytes), Duration::from_nanos(10))
                .expect("failed to write data");
        })
    });

    group.bench_function("read/check_bytes_true", |b| {
        b.iter(|| {
            let archived = unsafe { synchronizer.read::<HelloWorld>(true).unwrap() };
            assert_eq!(archived.version, data.version);
        })
    });

    group.bench_function("read/check_bytes_false", |b| {
        b.iter(|| {
            let archived = unsafe { synchronizer.read::<HelloWorld>(false).unwrap() };
            assert_eq!(archived.version, data.version);
        })
    });
}

#[cfg(unix)]
fn build_synchronizers_for_strategies() -> (
    Synchronizer<WyHash, LockDisabled, 1024, 1_000_000_000>,
    Synchronizer<WyHash, SingleWriter, 1024, 1_000_000_000>,
) {
    let disabled_path = "/dev/shm/mmap_sync_lock_disabled";
    let single_writer_path = "/dev/shm/mmap_sync_lock_single_writer";

    (
        Synchronizer::<WyHash, LockDisabled, 1024, 1_000_000_000>::with_params(
            disabled_path.as_ref(),
        ),
        Synchronizer::<WyHash, SingleWriter, 1024, 1_000_000_000>::with_params(
            single_writer_path.as_ref(),
        ),
    )
}

#[cfg(unix)]
pub fn bench_locked_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("synchronizer_locked_write");
    group.throughput(Throughput::Elements(1));

    let (mut synchronizer_disabled, mut synchronizer_single_writer) =
        build_synchronizers_for_strategies();
    let (data, _) = build_mock_data();

    group.bench_function("disabled", |b| {
        b.iter(|| {
            synchronizer_disabled
                .write(black_box(&data), Duration::from_nanos(10))
                .expect("failed to write data");
        })
    });

    group.bench_function("single_writer", |b| {
        b.iter(|| {
            synchronizer_single_writer
                .write(black_box(&data), Duration::from_nanos(10))
                .expect("failed to write data");
        })
    });
}

#[cfg(unix)]
pub fn bench_locked_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("synchronizer_locked_read");
    group.throughput(Throughput::Elements(1));

    let (mut synchronizer_disabled, mut synchronizer_single_writer) =
        build_synchronizers_for_strategies();
    let (data, _) = build_mock_data();

    // Populate data to make it available to read.
    synchronizer_disabled
        .write(&data, Duration::from_nanos(10))
        .expect("failed to populate initial data");
    synchronizer_single_writer
        .write(&data, Duration::from_nanos(10))
        .expect("failed to populate initial data");

    group.bench_function("disabled", |b| {
        b.iter(|| {
            let archived = unsafe { synchronizer_disabled.read::<HelloWorld>(false).unwrap() };
            assert_eq!(archived.version, data.version);
        })
    });

    group.bench_function("single_writer", |b| {
        b.iter(|| {
            let archived = unsafe {
                synchronizer_single_writer
                    .read::<HelloWorld>(false)
                    .unwrap()
            };
            assert_eq!(archived.version, data.version);
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, pprof::criterion::Output::Protobuf));
    targets = bench_synchronizer, bench_locked_reads, bench_locked_writes
}
criterion_main!(benches);
