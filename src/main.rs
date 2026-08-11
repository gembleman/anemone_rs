#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(feature = "benchmark")]
#[global_allocator]
static ALLOCATOR: anemone_rs::bench_mem::CountingAllocator =
    anemone_rs::bench_mem::CountingAllocator;

fn main() {
    anemone_rs::run();
}
