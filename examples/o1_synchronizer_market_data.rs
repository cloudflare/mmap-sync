use mmap_sync::locks::SingleWriter;
use mmap_sync::synchronizer::Synchronizer;
// We will explicitly use WyHash as our Hasher:
use wyhash::WyHash;

use rand::Rng;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex}; // <— using std::sync::Mutex
use std::thread;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use std::time::{Duration, Instant};

use bytecheck::CheckBytes;
use rkyv::{Archive, Deserialize, Serialize};
// (AlignedVec is not used, so you can safely remove it from imports)

/// Example struct for the data to be shared
#[derive(Archive, Deserialize, Serialize, Debug)]
#[archive_attr(derive(CheckBytes))]
pub struct BidAsk {
    pub side: String, // "buy" or "sell"
    pub exchange: String,
    pub symbol: String,
    pub price: f64,
    pub size: f64,
    pub timestamp: f64,
}

#[derive(Archive, Deserialize, Serialize, Debug)]
#[archive_attr(derive(CheckBytes))]
pub struct BestBidAsk {
    pub best_bid: BidAsk,
    pub best_offer: BidAsk,
}

const NUM_READERS: usize = 12;
const NUM_ITERATIONS: usize = 10_000;

/// Derive a path for shared memory. By default, /tmp/mmap_sync_latency
/// is used unless environment variable MMAPSYNC_BM_ROOTDIR is specified.

fn derive_shm_path(subpath: &str) -> PathBuf {
    const EV_NAME: &str = "MMAPSYNC_BM_ROOTDIR";
    const DEFAULT_ROOT: &str = "/tmp/mmap_sync_latency";

    let selected_root: String = match env::var(EV_NAME) {
        Ok(val) => {
            let requested_root = val.trim();
            if requested_root.is_empty() {
                DEFAULT_ROOT.into()
            } else {
                if let Ok(md) = fs::metadata(requested_root) {
                    if md.is_dir() {
                        requested_root.into()
                    } else {
                        eprintln!(
                            "Path in {EV_NAME} is not a directory; falling back to {DEFAULT_ROOT}"
                        );
                        DEFAULT_ROOT.into()
                    }
                } else {
                    eprintln!("Path in {EV_NAME} not accessible; falling back to {DEFAULT_ROOT}");
                    DEFAULT_ROOT.into()
                }
            }
        }
        Err(_) => DEFAULT_ROOT.into(),
    };

    // Instead of returning a String, build a PathBuf
    let path_str = format!("{selected_root}/{subpath}");
    PathBuf::from(path_str)
}

/// Generate random BestBidAsk data for the demonstration.
fn generate_random_best_bid_ask() -> BestBidAsk {
    let mut rng = rand::thread_rng();
    let now = SystemTime::now();
    // Calculate the duration since the Unix epoch
    let duration_since_epoch = now
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs_f64();

    BestBidAsk {
        best_bid: BidAsk {
            side: "buy".to_string(),
            exchange: "EXCH".to_string(),
            symbol: "TEST".to_string(),
            price: rng.gen_range(100.0..200.0),
            size: rng.gen_range(1.0..100.0),
            timestamp: duration_since_epoch,
        },
        best_offer: BidAsk {
            side: "sell".to_string(),
            exchange: "EXCH".to_string(),
            symbol: "TEST".to_string(),
            price: rng.gen_range(201.0..300.0),
            size: rng.gen_range(1.0..100.0),
            timestamp: duration_since_epoch,
        },
    }
}

/// Simple function to compute basic stats from a vector of latencies
/// (microseconds). Returns (count, min, max, mean, median, p95).
fn compute_stats(mut latencies_us: Vec<u64>) -> (usize, u64, u64, f64, f64, f64, f64) {
    let count = latencies_us.len();
    if count == 0 {
        return (0, 0, 0, 0.0, 0.0, 0.0, 0.0);
    }

    latencies_us.sort_unstable();
    let min = latencies_us[0];
    let max = latencies_us[count - 1];
    let sum: u128 = latencies_us.iter().map(|x| *x as u128).sum();
    let mean = sum as f64 / count as f64;
    let median = if count % 2 == 1 {
        latencies_us[count / 2] as f64
    } else {
        let mid1 = latencies_us[count / 2 - 1];
        let mid2 = latencies_us[count / 2];
        (mid1 as f64 + mid2 as f64) / 2.0
    };

    // 95th percentile index (0-based)
    let p95_idx = ((count as f64) * 0.95).ceil() as usize - 1;
    let p95_idx = p95_idx.clamp(0, count - 1);
    let p95 = latencies_us[p95_idx] as f64;

    // * p99
    let p99_idx = ((count as f64) * 0.99).ceil() as usize - 1;
    let p99_idx = p99_idx.clamp(0, count - 1);
    let p99 = latencies_us[p99_idx] as f64;

    (count, min, max, mean, median, p95, p99)
}

fn main() {
    type MySyncType = Synchronizer<WyHash, SingleWriter, 1024, 1_000_000_000>;

    // Create the Synchronizer for single-writer usage
    // NOTE: we call `with_params` instead of `new`
    let shm_path = derive_shm_path("best_bid_ask");
    // `shm_path` is now a PathBuf
    let shm_path = Arc::new(shm_path); // Arc<PathBuf>

    // Add a shared 'done' flag so readers can stop
    let done_flag = Arc::new(AtomicBool::new(false));

    // Shared data for storing latencies
    // Each thread (including writer) will push results into these vectors
    let writer_latencies = Arc::new(Mutex::new(Vec::new()));
    let reader_latencies = Arc::new(Mutex::new(Vec::new()));

    // ----------------------
    // Spawn the writer
    // ----------------------
    let done_flag_writer = Arc::clone(&done_flag);
    let writer_results = Arc::clone(&writer_latencies);
    let shm_path_clone = Arc::clone(&shm_path);
    let writer_handle = thread::spawn(move || {
        let mut synchronizer = MySyncType::with_params(shm_path_clone.as_os_str());

        for _i in 0..NUM_ITERATIONS {
            // let data = generate_random_best_bid_ask();
            // * generate 1k random BestBidAsk data
            let data = (0..1000)
                .into_iter()
                .map(|_| generate_random_best_bid_ask())
                .collect::<Vec<BestBidAsk>>();

            let start = Instant::now();
            // Attempt to write to the shared region
            synchronizer
                .write(&data, Duration::from_millis(100))
                .expect("Writer failed to write data");
            let elapsed = start.elapsed().as_micros() as u64;

            // Record writer's latency
            writer_results.lock().unwrap().push(elapsed);
        }

        // After finishing all writes, set the 'done' flag:
        done_flag_writer.store(true, Ordering::SeqCst);
        // Then writer thread ends
    });

    // ----------------------
    // Spawn 12 reader threads
    // ----------------------
    let mut reader_handles = Vec::new();
    for _r in 0..NUM_READERS {
        let reader_results: Arc<Mutex<Vec<u64>>> = Arc::clone(&reader_latencies);
        let shm_path_clone = Arc::clone(&shm_path);
        let done_flag_clone = Arc::clone(&done_flag);

        let handle = thread::spawn(move || {
            let mut synchronizer = MySyncType::with_params(shm_path_clone.as_os_str());
            // We'll store the last seen version
            let mut last_version = synchronizer.version().unwrap(); // The current version at start

            loop {
                // measure read
                let start = Instant::now();
                let current_version = synchronizer.version().unwrap();
                if current_version != last_version {
                    let archived = unsafe {
                        synchronizer
                            .read::<Vec<BestBidAsk>>(true)
                            .expect("Failed to read data")
                    };
                    let _ = &archived.first().unwrap().best_bid;

                    let elapsed = start.elapsed().as_micros() as u64;
                    last_version = current_version;

                    // store read latency
                    reader_results.lock().unwrap().push(elapsed);
                }

                // Sleep for a bit to avoid busy-waiting,
                thread::sleep(Duration::from_micros(100));
                // Check if writer is done AND there's no new version
                if done_flag_clone.load(Ordering::SeqCst) {
                    // One last check if a new version snuck in after done
                    // If so, loop again to read it
                    let latest = synchronizer.version().unwrap_or(last_version);
                    if latest == last_version {
                        break; // no new data, safe to exit
                    }
                }
            }
        });
        reader_handles.push(handle);
    }

    // Wait for writer and all readers to complete
    writer_handle.join().unwrap();
    for h in reader_handles {
        h.join().unwrap();
    }

    // Compute final stats
    let writer_data = writer_latencies.lock().unwrap().clone();
    let (w_count, w_min, w_max, w_mean, w_median, w_p95, w_p99) = compute_stats(writer_data);

    let reader_data = reader_latencies.lock().unwrap().clone();
    let (r_count, r_min, r_max, r_mean, r_median, r_p95, r_p99) = compute_stats(reader_data);

    println!("=== Final Latency Results ===");
    println!("Writer operations: {}", w_count);
    println!("  min:    {:.10} us", w_min);
    println!("  max:    {:.10} us", w_max);
    println!("  mean:   {:.10} us", w_mean);
    println!("  median: {:.10} us", w_median);
    println!("  p95:    {:.10} us", w_p95);
    println!("  p99:    {:.10} us", w_p99);

    println!("Readers (aggregate) operations: {}", r_count);
    println!("  min:    {:.10} us", r_min);
    println!("  max:    {:.10} us", r_max);
    println!("  mean:   {:.10} us", r_mean);
    println!("  median: {:.10} us", r_median);
    println!("  p95:    {:.10} us", r_p95);
    println!("  p99:    {:.10} us", r_p99);

    println!("Benchmark completed.");
    // * output the raw data to a file

    // writer_latencies.csv
    let writer_data = writer_latencies.lock().unwrap().clone();
    let mut w_csv = csv::Writer::from_path("writer_latencies.csv").unwrap();
    for w in writer_data {
        w_csv.write_record(&[w.to_string()]).unwrap();
    }

    // reader_latencies.csv
    let reader_data = reader_latencies.lock().unwrap().clone();
    let mut r_csv = csv::Writer::from_path("reader_latencies.csv").unwrap();
    for r in reader_data {
        r_csv.write_record(&[r.to_string()]).unwrap();
    }
}
