use std::cell::UnsafeCell;
use std::hint;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{
    AtomicBool,
    Ordering::{Acquire, Release},
};

pub struct SpinLock<T> {
    data: UnsafeCell<T>,
    locked: AtomicBool,
}

impl<T> SpinLock<T> {
    pub fn new(data: T) -> Self {
        Self {
            data: UnsafeCell::new(data),
            locked: AtomicBool::new(false),
        }
    }

    pub fn lock(&self) -> Guard<'_, T> {
        while self.locked.swap(true, Acquire) {
            hint::spin_loop()
        }

        Guard { lock: self }
    }

    fn unlock(&self) {
        self.locked.store(false, Release);
    }
}

unsafe impl<T> Send for SpinLock<T> {}
unsafe impl<T> Sync for SpinLock<T> {}

pub struct Guard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<'a, T> Deref for Guard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for Guard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for Guard<'a, T> {
    fn drop(&mut self) {
        self.lock.unlock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        sync::{atomic::Ordering::Relaxed, mpsc, Arc},
        thread,
        time::Duration,
    };

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_data(
            data in any::<String>()
        ) {
            let l = SpinLock::new(data.clone());

            thread::scope(|s| {
                s.spawn(|| {
                    assert_eq!(*l.lock(), data);
                });
            });

            assert_eq!(*l.lock(), data);
        }


        #[test]
        fn test_lock(
            data in any::<usize>()
        ) {
            let l = Arc::new(SpinLock::new(data));
            assert!(!l.locked.load(Relaxed));

            let l2 = l.clone();
            let (tx, rx) = mpsc::channel();

            thread::spawn(move || {
                // Acquire the lock
                let mut guard = l2.lock();
                assert!(l2.locked.load(Relaxed));

                // Signal that this thread has acquired the lock
                tx.send(()).unwrap();

                // Ensure that the lock is held across yield points
                thread::yield_now();

                // Hold the lock for some amount of time
                thread::sleep(Duration::from_micros(10));

                // Mutate the locked value
                *guard += 1;
            });

            // Wait to acquire the lock on this thread
            rx.recv().unwrap();

            let guard = l.lock();
            assert!(l.locked.load(Relaxed));

            // Check that the lock contains the updated value
            assert_eq!(*guard, data + 1);

            drop(guard);
            assert!(!l.locked.load(Relaxed));
        }


        #[test]
        fn test_thread_safety(num_threads in 2..100usize) {
            let l = SpinLock::new(0);

            thread::scope(|s| {
                for _ in 0..num_threads {
                    s.spawn(|| {
                        // Incerement the locked value
                        *l.lock() += 1;
                    });
                }
            });

            assert_eq!(*l.lock(), num_threads);
        }
    }
}
