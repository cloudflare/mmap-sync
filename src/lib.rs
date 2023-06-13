//! `mmap-sync` is a high-performance, concurrent data access library for Rust. It is designed to handle data access between a single writer process and multiple reader processes efficiently using memory-mapped files, wait-free synchronization, and zero-copy deserialization.
//!
//! ## Features
//!
//! - **Memory-mapped files**: This allows different processes to access the same memory space, bypassing the need for costly serialization and deserialization. As a result, `mmap-sync` provides fast, low-overhead data sharing between processes.
//!
//! - **Wait-free synchronization**: Inspired by [Linux kernel's Read-Copy-Update (RCU) pattern](https://www.kernel.org/doc/html/next/RCU/whatisRCU.html) and the [Left-Right concurrency control technique](https://github.com/pramalhe/ConcurrencyFreaks/blob/master/papers/left-right-2014.pdf). Write access to the data is managed by a single writer, with multiple readers able to access the data concurrently.
//!
//! - **Zero-copy deserialization**: Leveraging the [rkyv](https://rkyv.org/) library, `mmap-sync` achieves efficient data storage and retrieval. The templated type `T` for `Synchronizer` can be any Rust struct implementing specified `rkyv` traits.
//!
//! To get started with `mmap-sync`, please see the [examples](https://github.com/cloudflare/mmap-sync/tree/main/examples) provided.
mod data;
pub mod guard;
mod instance;
mod state;
pub mod synchronizer;
