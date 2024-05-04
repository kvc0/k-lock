#![doc = include_str!(concat!(env!("OUT_DIR"), "/README-rustdocified.md"))]

mod k_lock_mutex;
mod poison;

pub use k_lock_mutex::Mutex;
pub use k_lock_mutex::MutexGuard;
pub use poison::PoisonError;
pub use poison::TryLockError;
pub use poison::TryLockResult;
