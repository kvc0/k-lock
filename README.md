A mutual exclusion primitive useful for protecting shared data

This mutex will block threads waiting for the lock to become available. The
mutex can be created via a [`new`] constructor. Each mutex has a type parameter
which represents the data that it is protecting. The data can only be accessed
through the RAII guards returned from [`lock`] and [`try_lock`], which
guarantees that the data is only ever accessed when the mutex is locked.

# Difference from `std::sync::Mutex`
16 threads, short critical section expression: `*mutex.lock() += 1;` 
## AMD x86_64
![image](https://github.com/kvc0/k-lock/assets/3454741/43d61318-ceae-4dcd-8cd7-ab39c07f5913)

## AWS c7g.2xlarge aarch64
![image](https://github.com/kvc0/k-lock/assets/3454741/eca25d8b-cb66-4af1-8ac4-67149b9e0455)


## About
This mutex is optimized for brief critical sections. It has short spin cycles
to prevent overwork, and it aggressively wakes multiple waiters per unlock.

If there is concern about possible panic in a critical section, `std::sync::Mutex`
is the appropriate choice.

If critical sections are more than a few nanoseconds long, `std::sync::Mutex`
may be better. As always, profiling and measuring is important.

Much of this mutex implementation and its documentation is adapted with humble
gratitude from the venerable `std::sync::Mutex`.

# Poisoning
The mutex in this module does not implement poisoning.

This means that the [`lock`] and [`try_lock`] methods return a lock, whether
a mutex has been poisoned or not. If a panic can put your program in an invalid
state, you need to ensure that a possibly invalid invariant is not witnessed.
In these cases you should strongly consider using `std::sync::Mutex`.

It is expected that most uses of this lock will work with brief critical sections
that do not expose a significant risk of panic.

# Examples

```rust
use std::sync::Arc;
use std::thread;
use std::sync::mpsc::channel;
use k_lock::Mutex;

const N: usize = 10;

// Spawn a few threads to increment a shared variable (non-atomically), and
// let the main thread know once all increments are done.
//
// Here we're using an Arc to share memory among threads, and the data inside
// the Arc is protected with a mutex.
let data = Arc::new(Mutex::new(0));

let (tx, rx) = channel();
for _ in 0..N {
    let (data, tx) = (Arc::clone(&data), tx.clone());
    thread::spawn(move || {
        // The shared state can only be accessed once the lock is held.
        // Our non-atomic increment is safe because we're the only thread
        // which can access the shared state when the lock is held.
        //
        // We unwrap() the return value to assert that we are not expecting
        // threads to ever fail while holding the lock.
        let mut data = data.lock();
        *data += 1;
        if *data == N {
            tx.send(()).unwrap();
        }
        // the lock is unlocked here when `data` goes out of scope.
    });
}

rx.recv().unwrap();
```

To unlock a mutex guard sooner than the end of the enclosing scope,
either create an inner scope or drop the guard manually.

```rust
use std::sync::Arc;
use std::thread;
use k_lock::Mutex;

const N: usize = 3;

let data_mutex = Arc::new(Mutex::new(vec![1, 2, 3, 4]));
let res_mutex = Arc::new(Mutex::new(0));

let mut threads = Vec::with_capacity(N);
(0..N).for_each(|_| {
    let data_mutex_clone = Arc::clone(&data_mutex);
    let res_mutex_clone = Arc::clone(&res_mutex);

    threads.push(thread::spawn(move || {
        // Here we use a block to limit the lifetime of the lock guard.
        let result = {
            let mut data = data_mutex_clone.lock();
            // This is the result of some important and long-ish work.
            let result = data.iter().fold(0, |acc, x| acc + x * 2);
            data.push(result);
            result
            // The mutex guard gets dropped here, together with any other values
            // created in the critical section.
        };
        // The guard created here is a temporary dropped at the end of the statement, i.e.
        // the lock would not remain being held even if the thread did some additional work.
        *res_mutex_clone.lock() += result;
    }));
});

let mut data = data_mutex.lock();
// This is the result of some important and long-ish work.
let result = data.iter().fold(0, |acc, x| acc + x * 2);
data.push(result);
// We drop the `data` explicitly because it's not necessary anymore and the
// thread still has work to do. This allow other threads to start working on
// the data immediately, without waiting for the rest of the unrelated work
// to be done here.
//
// It's even more important here than in the threads because we `.join` the
// threads after that. If we had not dropped the mutex guard, a thread could
// be waiting forever for it, causing a deadlock.
// As in the threads, a block could have been used instead of calling the
// `drop` function.
drop(data);
// Here the mutex guard is not assigned to a variable and so, even if the
// scope does not end after this line, the mutex is still released: there is
// no deadlock.
*res_mutex.lock() += result;

threads.into_iter().for_each(|thread| {
    thread
        .join()
        .expect("The thread creating or execution failed !")
});

assert_eq!(*res_mutex.lock(), 800);
```
