use std::{cmp::max, ops::DerefMut, time::Instant};

use criterion::{criterion_group, measurement::WallTime, BenchmarkGroup, BenchmarkId, Criterion};
use k_lock::Mutex;

#[allow(clippy::expect_used)] // this is a benchmark, lints like this don't matter.
fn contention(c: &mut Criterion) {
    for thread_count in [1, 2, 4, 8, 16] {
        let mut group = c.benchmark_group("Mutex");
        group.nresamples(800000);
        group.throughput(criterion::Throughput::Elements(1));

        bench(
            "k-lock",
            &mut group,
            thread_count,
            k_lock::Mutex::new(0_usize),
        );
        bench(
            "std",
            &mut group,
            thread_count,
            std::sync::Mutex::new(0_usize),
        );
    }
}

fn bench(
    name: &str,
    group: &mut BenchmarkGroup<'_, WallTime>,
    thread_count: usize,
    mutex: impl Lock<usize>,
) {
    group.bench_function(BenchmarkId::new(name, thread_count), |bencher| {
        bencher.iter_custom(|iterations| {
            let iterations_per_thread = max(1, iterations as usize / thread_count);

            let start = Instant::now();
            std::thread::scope(|scope| {
                for _ in 0..thread_count {
                    scope.spawn(|| {
                        for _ in 0..iterations_per_thread {
                            *mutex.lock() += 1;
                        }
                    });
                }
            });

            start.elapsed()
        });
    });
}

trait Lock<T>: Sync {
    fn lock(&self) -> impl DerefMut<Target = T>;
}

impl<T: Send> Lock<T> for Mutex<T> {
    #[inline]
    fn lock(&self) -> impl DerefMut<Target = T> {
        self.lock()
    }
}

impl<T: Send> Lock<T> for std::sync::Mutex<T> {
    #[inline]
    fn lock(&self) -> impl DerefMut<Target = T> {
        self.lock().expect("it should work")
    }
}

criterion_group!(benches, contention);
