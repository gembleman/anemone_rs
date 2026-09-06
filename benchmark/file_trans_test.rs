pub use anemone_rs::file_trans;
pub use anemone_rs::file_trans::benchmark_support::{
    BoundedTranslationCache, read_input_line, translate_eztrans_window, validate_and_count_reader,
};
pub use anemone_rs::file_trans::{
    FileTranslationProgress, FileTranslationRequest, FileTranslationSupervisor,
    FileTranslationTask, WriteType,
};
pub use anemone_rs::translation;

#[path = "mem.rs"]
mod mem;

#[global_allocator]
static ALLOCATOR: mem::CountingAllocator = mem::CountingAllocator;

#[path = "file_trans/mod.rs"]
mod benchmarks;

#[path = "file_trans/worker.rs"]
mod worker_benchmarks;

#[path = "translation_cache_bench.rs"]
mod translation_cache_benchmarks;

#[path = "postprocess_bench.rs"]
mod postprocess_benchmarks;

#[path = "file_trans_workload_bench.rs"]
mod file_trans_workload_benchmarks;
