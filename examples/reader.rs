mod common;

use common::HelloWorld;
use mmap_sync::synchronizer::Synchronizer;

fn main() {
    // Initialize the Synchronizer
    let mut synchronizer = Synchronizer::new("/tmp/hello_world".as_ref());

    // Read data from shared memory
    let data = unsafe { synchronizer.read::<HelloWorld>(false) }.expect("failed to read data");

    // Access fields of the struct
    println!("version: {} | messages: {:?}", data.version, data.messages);
}
