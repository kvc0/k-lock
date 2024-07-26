use std::{
    cell::UnsafeCell,
    hint::spin_loop,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::addr_of,
    sync::atomic::{AtomicU32, Ordering},
};

use atomic_wait::wake_one;

use crate::poison::{self, LockResult, TryLockError, TryLockResult};

const UNLOCKED: u32 = 0;
const LOCKED: u32 = 1;
const CONTENDED: u32 = 2;
const EXTRA_CONTENDED: u32 = 3;

/// A mutual exclusion primitive useful for protecting shared data
///
/// This mutex will block threads waiting for the lock to become available. The
/// mutex can be created via a [`new`] constructor. Each mutex has a type parameter
/// which represents the data that it is protecting. The data can only be accessed
/// through the RAII guards returned from [`lock`] and [`try_lock`], which
/// guarantees that the data is only ever accessed when the mutex is locked.
///
/// # Difference from `std::sync::Mutex`
/// This mutex is optimized for brief critical sections. It has short spin cycles
/// to prevent overwork, and it aggressively wakes multiple waiters per unlock.
///
/// If there is concern about possible panic in a critical section, `std::sync::Mutex`
/// is the appropriate choice.
///
/// If critical sections are more than a few nanoseconds long, `std::sync::Mutex`
/// may be better. As always, profiling and measuring is important.
///
/// Much of this mutex implementation and its documentation is adapted with humble
/// gratitude from the venerable `std::sync::Mutex`.
///
/// # Poisoning
///
/// The mutex in this module uses the poisoning strategy from `std::sync::Mutex`.
///
/// [`new`]: Self::new
/// [`lock`]: Self::lock
/// [`try_lock`]: Self::try_lock
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use std::thread;
/// use std::sync::mpsc::channel;
/// use k_lock::Mutex;
///
/// const N: usize = 10;
///
/// // Spawn a few threads to increment a shared variable (non-atomically), and
/// // let the main thread know once all increments are done.
/// //
/// // Here we're using an Arc to share memory among threads, and the data inside
/// // the Arc is protected with a mutex.
/// let data = Arc::new(Mutex::new(0));
///
/// let (tx, rx) = channel();
/// for _ in 0..N {
///     let (data, tx) = (Arc::clone(&data), tx.clone());
///     thread::spawn(move || {
///         // The shared state can only be accessed once the lock is held.
///         // Our non-atomic increment is safe because we're the only thread
///         // which can access the shared state when the lock is held.
///         //
///         // We unwrap() the return value to assert that we are not expecting
///         // threads to ever fail while holding the lock.
///         let mut data = data.lock().unwrap();
///         *data += 1;
///         if *data == N {
///             tx.send(()).unwrap();
///         }
///         // the lock is unlocked here when `data` goes out of scope.
///     });
/// }
///
/// rx.recv().unwrap();
/// ```
///
/// To unlock a mutex guard sooner than the end of the enclosing scope,
/// either create an inner scope or drop the guard manually.
///
/// ```
/// use std::sync::Arc;
/// use std::thread;
/// use k_lock::Mutex;
///
/// const N: usize = 3;
///
/// let data_mutex = Arc::new(Mutex::new(vec![1, 2, 3, 4]));
/// let res_mutex = Arc::new(Mutex::new(0));
///
/// let mut threads = Vec::with_capacity(N);
/// (0..N).for_each(|_| {
///     let data_mutex_clone = Arc::clone(&data_mutex);
///     let res_mutex_clone = Arc::clone(&res_mutex);
///
///     threads.push(thread::spawn(move || {
///         // Here we use a block to limit the lifetime of the lock guard.
///         let result = {
///             let mut data = data_mutex_clone.lock().unwrap();
///             // This is the result of some important and long-ish work.
///             let result = data.iter().fold(0, |acc, x| acc + x * 2);
///             data.push(result);
///             result
///             // The mutex guard gets dropped here, together with any other values
///             // created in the critical section.
///         };
///         // The guard created here is a temporary dropped at the end of the statement, i.e.
///         // the lock would not remain being held even if the thread did some additional work.
///         *res_mutex_clone.lock().unwrap() += result;
///     }));
/// });
///
/// let mut data = data_mutex.lock().unwrap();
/// // This is the result of some important and long-ish work.
/// let result = data.iter().fold(0, |acc, x| acc + x * 2);
/// data.push(result);
/// // We drop the `data` explicitly because it's not necessary anymore and the
/// // thread still has work to do. This allow other threads to start working on
/// // the data immediately, without waiting for the rest of the unrelated work
/// // to be done here.
/// //
/// // It's even more important here than in the threads because we `.join` the
/// // threads after that. If we had not dropped the mutex guard, a thread could
/// // be waiting forever for it, causing a deadlock.
/// // As in the threads, a block could have been used instead of calling the
/// // `drop` function.
/// drop(data);
/// // Here the mutex guard is not assigned to a variable and so, even if the
/// // scope does not end after this line, the mutex is still released: there is
/// // no deadlock.
/// *res_mutex.lock().unwrap() += result;
///
/// threads.into_iter().for_each(|thread| {
///     thread
///         .join()
///         .expect("The thread creating or execution failed !")
/// });
///
/// assert_eq!(*res_mutex.lock().unwrap(), 800);
/// ```
pub struct Mutex<T: ?Sized> {
    futex: AtomicU32,
    lock_epoch: AtomicU32,
    poison: poison::Flag,
    data: UnsafeCell<T>,
}

impl<T: ?Sized> Mutex<T> {
    /// Acquires a mutex, blocking the current thread until it is able to do so.
    ///
    /// This function will block the local thread until it is available to acquire
    /// the mutex. Upon returning, the thread is the only thread with the lock
    /// held. An RAII guard is returned to allow scoped unlock of the lock. When
    /// the guard goes out of scope, the mutex will be unlocked.
    ///
    /// The exact behavior on locking a mutex in the thread which already holds
    /// the lock is left unspecified. However, this function will not return on
    /// the second call (it might panic or deadlock, for example).
    ///
    /// # Errors
    ///
    /// If another user of this mutex panicked while holding the mutex, then
    /// this call will return an error once the mutex is acquired.
    ///
    /// # Panics
    ///
    /// This function might panic when called if the lock is already held by
    /// the current thread. It also might not. Don't try it!
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use std::thread;
    /// use k_lock::Mutex;
    ///
    /// let mutex = Arc::new(Mutex::new(0));
    /// let c_mutex = Arc::clone(&mutex);
    ///
    /// thread::spawn(move || {
    ///     *c_mutex.lock().unwrap() = 10;
    /// }).join().expect("thread::spawn failed");
    /// assert_eq!(*mutex.lock().unwrap(), 10);
    /// ```
    #[inline]
    pub fn lock(&self) -> LockResult<MutexGuard<T>> {
        if self
            .futex
            .compare_exchange(UNLOCKED, LOCKED, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            self.lock_epoch.fetch_add(1, Ordering::Relaxed);
            return MutexGuard::new(self);
        }
        self.lock_contended()
    }

    /// Move this out so it does not bloat asm and reduce the likelihood of lock() being inlined.
    #[cold]
    #[allow(clippy::comparison_chain)] // I prefer it this way in this case because of the semantic meaning
    fn lock_contended(&self) -> LockResult<MutexGuard<T>> {
        loop {
            let state = self.spin();
            // when locking under contention you have to stay contended or you may leak wakes
            let expect = if state < CONTENDED {
                if self.futex.swap(CONTENDED, Ordering::Acquire) == UNLOCKED {
                    self.lock_epoch.fetch_add(1, Ordering::Relaxed);
                    return MutexGuard::new(self);
                }
                CONTENDED
            } else if state == CONTENDED {
                if self.futex.swap(EXTRA_CONTENDED, Ordering::Acquire) == UNLOCKED {
                    self.lock_epoch.fetch_add(1, Ordering::Release);
                    return MutexGuard::new(self);
                }
                EXTRA_CONTENDED
            } else {
                // we've already promoted to extra contended. We're... extra contended.
                EXTRA_CONTENDED
            };
            atomic_wait::wait(&self.futex, expect);
        }
    }

    /// Move this out so it does not bloat asm.
    #[cold]
    fn spin(&self) -> u32 {
        let mut spin = 400;
        let mut epoch = 0;
        loop {
            let v = if 200 < spin {
                self.futex.load(Ordering::Relaxed)
            } else {
                self.futex.load(Ordering::Acquire)
            };
            if v != LOCKED || spin == 0 {
                break v;
            }
            let now = if 200 < spin {
                self.lock_epoch.load(Ordering::Relaxed)
            } else {
                self.lock_epoch.load(Ordering::Acquire)
            };
            if now != epoch {
                // Refresh the spin because this lock is making timely progress.
                epoch = now;
                spin = 400;
                // This might be too aggressive - it drops latency for small critical sections
                // by keeping this thread out of futex, but yield_now is not free either.
                // Adding a yield to the spin refresh dropped contended latency by over 25%,
                // but there may be other heuristics that outperform this.
                std::thread::yield_now();
            }
            spin_loop();
            spin -= 1;
        }
    }

    /// Attempts to acquire this lock.
    ///
    /// If the lock could not be acquired at this time, then [`Err`] is returned.
    /// Otherwise, an RAII guard is returned. The lock will be unlocked when the
    /// guard is dropped.
    ///
    /// This function does not block.
    ///
    /// # Errors
    ///
    /// If the mutex could not be acquired because it is already locked, then
    /// this call will return the [`WouldBlock`] error.
    ///
    /// [`WouldBlock`]: TryLockError::WouldBlock
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use std::thread;
    /// use k_lock::Mutex;
    ///
    /// let mutex = Arc::new(Mutex::new(0));
    /// let c_mutex = Arc::clone(&mutex);
    ///
    /// thread::spawn(move || {
    ///     let mut lock = c_mutex.try_lock();
    ///     if let Ok(ref mut mutex) = lock {
    ///         **mutex = 10;
    ///     } else {
    ///         println!("try_lock failed");
    ///     }
    /// }).join().expect("thread::spawn failed");
    /// assert_eq!(*mutex.lock().unwrap(), 10);
    /// ```
    #[inline]
    pub fn try_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        match self
            .futex
            .compare_exchange(UNLOCKED, LOCKED, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => Ok(MutexGuard::new(self)?),
            Err(_) => Err(TryLockError::WouldBlock),
        }
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// Since this call borrows the `Mutex` mutably, no actual locking needs to
    /// take place -- the mutable borrow statically guarantees no locks exist.
    ///
    /// # Errors
    ///
    /// PoisonError if a thread has previously paniced while holding this mutex.
    ///
    /// # Examples
    ///
    /// ```
    /// use k_lock::Mutex;
    ///
    /// let mut mutex = Mutex::new(0);
    /// *mutex.get_mut().unwrap() = 10;
    /// assert_eq!(*mutex.lock().unwrap(), 10);
    /// ```
    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        let data = self.data.get_mut();
        poison::map_result(self.poison.borrow(), |()| data)
    }
}

impl<T> Mutex<T> {
    /// Creates a new mutex in an unlocked state ready for use.
    ///
    /// # Examples
    ///
    /// ```
    /// use k_lock::Mutex;
    ///
    /// let mutex = Mutex::new(0);
    /// ```
    #[inline]
    pub const fn new(data: T) -> Self {
        Self {
            data: UnsafeCell::new(data),
            lock_epoch: AtomicU32::new(0),
            poison: poison::Flag::new(),
            futex: AtomicU32::new(UNLOCKED),
        }
    }
}

impl<T> From<T> for Mutex<T> {
    /// Creates a new mutex in an unlocked state ready for use.
    /// This is equivalent to [`Mutex::new`].
    fn from(t: T) -> Self {
        Mutex::new(t)
    }
}

impl<T: ?Sized + Default> Default for Mutex<T> {
    /// Creates a `Mutex<T>`, with the `Default` value for T.
    fn default() -> Mutex<T> {
        Mutex::new(Default::default())
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("Mutex");
        match self.try_lock() {
            Ok(guard) => {
                d.field("data", &&*guard);
            }
            Err(TryLockError::Poisoned(err)) => {
                d.field("data", &&**err.get_ref());
            }
            Err(TryLockError::WouldBlock) => {
                d.field("data", &format_args!("<locked>"));
            }
        }
        d.field("poisoned", &self.poison.get());
        d.finish_non_exhaustive()
    }
}

// these are the only places where `T: Send` matters; all other
// functionality works fine on a single thread.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

// negative impls are not stable yet...
// impl<T: ?Sized> !Send for MutexGuard<'_, T> {}
unsafe impl<T: ?Sized + Sync> Sync for MutexGuard<'_, T> {}

/// An RAII implementation of a "scoped lock" of a mutex. When this structure is
/// dropped (falls out of scope), the lock will be unlocked.
///
/// The data protected by the mutex can be accessed through this guard via its
/// [`Deref`] and [`DerefMut`] implementations.
///
/// This structure is created by the [`lock`] and [`try_lock`] methods on
/// [`Mutex`].
///
/// [`lock`]: Mutex::lock
/// [`try_lock`]: Mutex::try_lock
#[must_use = "if unused the Mutex will immediately unlock"]
#[clippy::has_significant_drop]
pub struct MutexGuard<'a, T: ?Sized + 'a> {
    lock: &'a Mutex<T>,
    poison: poison::Guard,
    _phantom: PhantomUnsend,
}

impl<'a, T: ?Sized> MutexGuard<'a, T> {
    fn new(lock: &'a Mutex<T>) -> LockResult<Self> {
        poison::map_result(lock.poison.guard(), |guard| Self {
            lock,
            poison: guard,
            _phantom: PhantomData,
        })
    }
}

pub type PhantomUnsend = PhantomData<std::sync::MutexGuard<'static, ()>>;

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.poison.done(&self.poison);
        let released = self.lock.futex.swap(UNLOCKED, Ordering::Release);
        if released == CONTENDED {
            wake_one(addr_of!(self.lock.futex));
        } else if released == EXTRA_CONTENDED {
            wake_one(addr_of!(self.lock.futex));
            wake_one(addr_of!(self.lock.futex));
        }
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized + std::fmt::Display> std::fmt::Display for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::Mutex;

    #[test]
    fn poisoned() {
        let m = Arc::new(Mutex::new(()));
        let mt = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = mt.lock().expect("lock must succeed");
            panic!("bail while locked");
        })
        .join();
        match m.lock() {
            Ok(_) => panic!("must not lock"),
            Err(_poison) => {
                // it is poisoned
            }
        };
    }
}
