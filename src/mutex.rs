use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{
        AtomicU32,
        Ordering::{Acquire, Relaxed, Release},
    },
};

use atomic_wait::{wait, wake_one};

const UNLOCKED: u32 = 0;
const LOCKED: u32 = 1;
const LOCKED_WITH_WAITERS: u32 = 2;

pub struct Mutex<T> {
    state: AtomicU32,
    value: UnsafeCell<T>,
}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            state: AtomicU32::new(UNLOCKED),
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> MutexGuard<T> {
        if self
            .state
            .compare_exchange(UNLOCKED, LOCKED, Acquire, Relaxed)
            .is_err()
        {
            while self.state.swap(LOCKED_WITH_WAITERS, Acquire) != UNLOCKED {
                wait(&self.state, LOCKED_WITH_WAITERS);
            }
        }

        MutexGuard { mutex: self }
    }
}

unsafe impl<T> Sync for Mutex<T> where T: Send {}

pub struct MutexGuard<'a, T> {
    pub(crate) mutex: &'a Mutex<T>,
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        if self.mutex.state.swap(UNLOCKED, Release) == LOCKED_WITH_WAITERS {
            wake_one(&self.mutex.state);
        }
    }
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.value.get() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use std::thread;

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_lock(
            value_1 in any::<String>(),
            value_2 in any::<String>(),
        ) {
            let mutex = Mutex::new(value_1.clone());

            let mut guard = mutex.lock();
            prop_assert_eq!(guard.deref(), &value_1);
            *guard = value_2.clone();
            drop(guard);

            let guard = mutex.lock();
            prop_assert_eq!(guard.deref(), &value_2);
        }

        #[test]
        fn test_lock_thread_safe(
            value_1 in any::<String>(),
            value_2 in any::<String>(),
            threads in 1..10,
        ) {
            let mutex = Arc::new(Mutex::new(value_1.clone()));

            thread::scope(|s| {
                for _ in 0..threads {
                    let mutex = mutex.clone();
                    let value_2 = value_2.clone();

                    s.spawn(move || {
                        let mut guard = mutex.lock();
                        *guard = value_2.clone();
                    });
                }
            });

            let guard = mutex.lock();
            prop_assert_eq!(guard.deref(), &value_2);
        }

        #[test]
        fn test_lock_correct_value(
            value in 1..10_000usize,
            threads in 1..10usize,
        ) {
            let mutex = Arc::new(Mutex::new(0));

            thread::scope(|s| {
                for _ in 0..threads {
                    let mutex = mutex.clone();

                    s.spawn(move || {
                        for _ in 0..value {
                            *mutex.lock() += 1;
                        }
                    });
                }
            });

            let guard = mutex.lock();
            prop_assert_eq!(*guard, value * threads);
        }
    }
}
