use criterion::criterion_main;

mod benchmarks;

criterion_main! {
    benchmarks::k_lock_bench::benches,
}
