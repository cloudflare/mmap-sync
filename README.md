# mmap-sync
[![docs.rs](https://docs.rs/mmap-sync/badge.svg)](https://docs.rs/mmap-sync)
[![crates.io](https://img.shields.io/crates/v/mmap-sync.svg)](https://crates.io/crates/rkyv)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue)](LICENSE)

`mmap-sync` is a Rust crate designed to manage high-performance, concurrent data access between a single writer process and multiple reader processes, leveraging the benefits of memory-mapped files, wait-free synchronization, and zero-copy deserialization.
We're using `mmap-sync` for large-scale machine learning, detailed in our blog post: ["Every Request, Every Microsecond: Scalable machine learning at Cloudflare"](https://example.com/blogpost).

## Overview
At the core of `mmap-sync` is a `Synchronizer` structure that offers a simple interface with "write" and "read" methods, allowing users to read and write any Rust struct (`T`) that implements or derives certain rkyv traits.

```rust
impl Synchronizer {
    /// Write a given `entity` into the next available memory mapped file.
    pub fn write<T>(&mut self, entity: &T, grace_duration: Duration) -> Result<(usize, bool), SynchronizerError> {
        …
    }

    /// Reads and returns `entity` struct from mapped memory wrapped in `ReadResult`
    pub fn read<T>(&mut self) -> Result<ReadResult<T>, SynchronizerError> {
        …
    }
}
```

Data is stored in shared mapped memory, allowing the `Synchronizer` to "write" and "read" from it concurrently.
This makes `mmap-sync` a highly efficient and flexible tool for managing shared, concurrent data access.

## Mapped Memory
The use of memory-mapped files offers several advantages over other inter-process communication (IPC) mechanisms.
It allows different processes to access the same memory space, bypassing the need for costly serialization and deserialization.
This design allows `mmap-sync` to provide extremely fast, low-overhead data sharing between processes.

## Wait-free Synchronization
Our wait-free data access pattern draws inspiration from [Linux kernel's Read-Copy-Update (RCU) pattern](https://www.kernel.org/doc/html/next/RCU/whatisRCU.html) and the [Left-Right concurrency control technique](https://github.com/pramalhe/ConcurrencyFreaks/blob/master/papers/left-right-2014.pdf).
In our solution, we maintain two copies of the data in separate memory-mapped files.
Write access to this data is managed by a single writer, with multiple readers able to access the data concurrently.

We store the synchronization state, which coordinates access to these data copies, in a third memory-mapped file, referred to as "state".
This file contains an atomic 64-bit integer, which represents an `InstanceVersion` and a pair of additional atomic 32-bit variables, tracking the number of active readers for each data copy.
The `InstanceVersion` consists of the currently active data file index (1 bit), the data size (39 bits, accommodating data sizes up to 549 GB), and a data checksum (24 bits).

## Zero-copy Deserialization
To efficiently store and fetch data, `mmap-sync` utilizes zero-copy deserialization with the help of the [rkyv](https://rkyv.org/) library, directly referencing bytes in the serialized form.
This significantly reduces the time and memory required to access and use data.
The templated type `T` for `Synchronizer` can be any Rust struct implementing specified `rkyv` traits.

## Getting Started
To use `mmap-sync`, add it to your `Cargo.toml` under `[dependencies]`:
```toml
[dependencies]
mmap-sync = "1.0.0"
```
Then, import `mmap-sync` in your Rust program:
```rust
use mmap_sync::synchronizer::Synchronizer;
```

Check out the provided examples for detailed usage:
* [Writer process example](examples/writer.rs): This example demonstrates how to define a Rust struct and write it into shared memory using `mmap-sync`.
* [Reader process example](examples/reader.rs): This example shows how to read data written into shared memory by a writer process.

These examples share a [common](examples/common.rs) module that defines the data structure being written and read.

To run these examples, follow these steps:

1. Open a terminal and navigate to your project directory.
2. Execute the writer example with the command `cargo run --example writer`.
3. In the same way, run the reader example using `cargo run --example reader`.

Upon successful execution of these examples, the terminal output should resemble:
```shell
# Writer example
written: 36 bytes | reset: false
# Reader example
version: 7 messages: ["Hello", "World", "!"]
```

Moreover, the following files will be created:
```shell
$ stat -c '%A %s %n' /tmp/hello_world_*
-rw-r----- 36 /tmp/hello_world_data_0
-rw-r----- 36 /tmp/hello_world_data_1
-rw-rw---- 16 /tmp/hello_world_state
```

With these steps, you can start utilizing `mmap-sync` in your Rust applications for efficient concurrent data access across processes.