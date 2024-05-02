#![doc = include_str!(concat!(env!("OUT_DIR"), "/README-rustdocified.md"))]

mod k_lock_mutex;

pub use k_lock_mutex::Mutex;
pub use k_lock_mutex::MutexGuard;
pub use k_lock_mutex::TryLockError;
pub use k_lock_mutex::TryLockResult;
