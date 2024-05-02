use std::{cmp::max, ops::DerefMut, time::Instant};

use criterion::{criterion_group, measurement::WallTime, BenchmarkGroup, BenchmarkId, Criterion};
use k_lock::Mutex;

#[allow(clippy::expect_used)] // this is a benchmark, lints like this don't matter.
fn contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("Mutex");
    group.nresamples(800000);
    for thread_count in [
        1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 32,
    ] {
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
        bench(
            "parking_lot",
            &mut group,
            thread_count,
            parking_lot::const_mutex(0_usize),
        );
    }
}

fn bench(
    name: &str,
    group: &mut BenchmarkGroup<'_, WallTime>,
    thread_count: usize,
    mutex: impl Lock<usize>,
) {
    group.bench_with_input(
        BenchmarkId::new(name, thread_count),
        &thread_count,
        |bencher, thread_count| {
            bencher.iter_custom(|iterations| {
                let iterations_per_thread = max(1, iterations as usize / thread_count);

                let start = Instant::now();
                std::thread::scope(|scope| {
                    for _ in 0..*thread_count {
                        scope.spawn(|| {
                            for _ in 0..iterations_per_thread {
                                *mutex.lock() += 1;
                            }
                        });
                    }
                });

                start.elapsed()
            });
        },
    );
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

impl<T: Send> Lock<T> for parking_lot::Mutex<T> {
    #[inline]
    fn lock(&self) -> impl DerefMut<Target = T> {
        self.lock()
    }
}

criterion_group!(benches, contention);
