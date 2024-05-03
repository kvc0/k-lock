use std::{cmp::max, collections::HashMap, ops::DerefMut, time::Instant};

use criterion::{criterion_group, measurement::WallTime, BenchmarkGroup, BenchmarkId, Criterion};
use k_lock::Mutex;

#[allow(clippy::expect_used)] // this is a benchmark, lints like this don't matter.
fn contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("hashmap critical section");
    group.nresamples(800000);
    for thread_count in [
        1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 32,
    ] {
        bench_map(
            "k-lock",
            &mut group,
            thread_count,
            k_lock::Mutex::new(HashMap::new()),
        );
        bench_map(
            "std",
            &mut group,
            thread_count,
            std::sync::Mutex::new(HashMap::new()),
        );
        // Parking lot is pretty slow, but it's here in case you are curious.
        // bench_map(
        //     "parking_lot",
        //     &mut group,
        //     thread_count,
        //     parking_lot::const_mutex(HashMap::new()),
        // );
    }
    group.finish();

    let mut group = c.benchmark_group("increment critical section");
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
        // Parking lot is pretty slow, but it's here in case you are curious.
        // bench(
        //     "parking_lot",
        //     &mut group,
        //     thread_count,
        //     parking_lot::const_mutex(0_usize),
        // );
    }
    group.finish();
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

fn bench_map(
    name: &str,
    group: &mut BenchmarkGroup<'_, WallTime>,
    thread_count: usize,
    mutex: impl Lock<HashMap<&'static str, &'static str>>,
) {
    fn test_str(i: usize) -> &'static str {
        match i % 10 {
            0 => "0",
            1 => "1",
            2 => "2",
            3 => "3",
            4 => "4",
            5 => "5",
            6 => "6",
            7 => "7",
            8 => "8",
            _ => "9",
        }
    }

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
                            for i in 0..iterations_per_thread {
                                mutex.lock().insert(test_str(i), test_str(i + 1));
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
