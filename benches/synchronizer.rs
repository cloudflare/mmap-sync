use std::time::Duration;

use bytecheck::CheckBytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use mmap_sync::synchronizer::Synchronizer;
use pprof::criterion::PProfProfiler;
use rkyv::{Archive, Deserialize, Serialize};

/// Example data-structure shared between writer and reader(s)
#[derive(Archive, Deserialize, Serialize, Debug, PartialEq)]
#[archive_attr(derive(CheckBytes))]
pub struct HelloWorld {
    pub version: u32,
    pub messages: Vec<String>,
}

pub fn bench_synchronizer(c: &mut Criterion) {
    let mut synchronizer = Synchronizer::new("/dev/shm/hello_world".as_ref());
    let data = HelloWorld {
        version: 7,
        messages: vec!["Hello".to_string(), "World".to_string(), "!".to_string()],
    };
    let bytes = rkyv::to_bytes::<HelloWorld, 1024>(&data).unwrap();

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

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, pprof::criterion::Output::Protobuf));
    targets = bench_synchronizer
}
criterion_main!(benches);
