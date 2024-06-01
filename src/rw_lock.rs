use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{
        AtomicU32,
        Ordering::{Acquire, Relaxed, Release},
    },
};

use atomic_wait::{wait, wake_all, wake_one};

const WRITE_LOCKED: u32 = u32::MAX;

pub struct RwLock<T> {
    state: AtomicU32,
    writer_wake_counter: AtomicU32,
    value: UnsafeCell<T>,
}

impl<T> RwLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            // The number of read locks multiplied by 2 (even numbers),
            // plus 1 if there is a waiting writer (odd numbers) to prevent write lock starvation,
            // or `u32::MAX` if there is a write lock.
            state: AtomicU32::new(0),
            writer_wake_counter: AtomicU32::new(0),
            value: UnsafeCell::new(value),
        }
    }

    pub fn write(&self) -> WriteGuard<T> {
        let mut s = self.state.load(Relaxed);

        loop {
            if s <= 1 {
                // Try to acquire a writer.
                match self
                    .state
                    .compare_exchange(s, WRITE_LOCKED, Acquire, Relaxed)
                {
                    Ok(_) => return WriteGuard { rw_lock: self },
                    Err(e) => {
                        s = e;
                        continue;
                    }
                }
            }

            if s % 2 == 0 {
                // There are currently readers, so block new readers by incrementing the state to an odd number.
                match self.state.compare_exchange(s, s + 1, Relaxed, Relaxed) {
                    Ok(_) => {}
                    Err(e) => {
                        s = e;
                        continue;
                    }
                }
            }

            // If there are still readers, wait until a writer can be acquired.
            let w = self.writer_wake_counter.load(Acquire);
            s = self.state.load(Relaxed);
            if s >= 2 {
                wait(&self.writer_wake_counter, w);
                s = self.state.load(Relaxed);
            }
        }
    }

    pub fn read(&self) -> ReadGuard<T> {
        let mut s = self.state.load(Acquire);

        loop {
            if s % 2 == 0 {
                match self.state.compare_exchange(s, s + 2, Acquire, Relaxed) {
                    Ok(_) => return ReadGuard { rw_lock: self },
                    Err(e) => s = e,
                }
            } else {
                wait(&self.state, s);
                s = self.state.load(Relaxed);
            }
            // if s == WRITE_LOCKED {
            //     wait(&self.state, WRITE_LOCKED);
            //     s = self.state.load(Relaxed);
            // } else if s == WRITE_LOCKED - 1 {
            //     wait(&self.state, WRITE_LOCKED - 1);
            //     s = self.state.load(Relaxed);
            // } else {
            //     match self.state.compare_exchange(s, s + 1, Acquire, Relaxed) {
            //         Ok(_) => return ReadGuard { rw_lock: self },
            //         Err(e) => s = e,
            //     }
            // }
        }
    }
}

unsafe impl<T> Sync for RwLock<T> where T: Send {}

pub struct WriteGuard<'a, T> {
    pub(crate) rw_lock: &'a RwLock<T>,
}

impl<T> Drop for WriteGuard<'_, T> {
    fn drop(&mut self) {
        self.rw_lock.state.store(0, Release);
        self.rw_lock.writer_wake_counter.fetch_add(1, Release);
        wake_one(&self.rw_lock.writer_wake_counter);
        wake_all(&self.rw_lock.state);
    }
}

impl<T> Deref for WriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.rw_lock.value.get() }
    }
}

impl<T> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.rw_lock.value.get() }
    }
}

pub struct ReadGuard<'a, T> {
    pub(crate) rw_lock: &'a RwLock<T>,
}

impl<T> Drop for ReadGuard<'_, T> {
    fn drop(&mut self) {
        if self.rw_lock.state.fetch_sub(2, Release) == 3 {
            // The last reader is releasing its lock, and there is a waiting writer, so wake one writer.
            self.rw_lock.writer_wake_counter.fetch_add(1, Release);
            wake_one(&self.rw_lock.writer_wake_counter);
        }
    }
}

impl<T> Deref for ReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.rw_lock.value.get() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{mpsc::channel, Arc, Barrier};
    use std::thread;

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_write(
            value_1 in any::<String>(),
            value_2 in any::<String>(),
        ) {
            let rw_lock = RwLock::new(value_1.clone());

            let mut guard = rw_lock.write();
            prop_assert_eq!(guard.deref(), &value_1);
            *guard = value_2.clone();
            drop(guard);

            let guard = rw_lock.write();
            prop_assert_eq!(guard.deref(), &value_2);
            drop(guard);

            let guard = rw_lock.read();
            prop_assert_eq!(guard.deref(), &value_2);
        }

        #[test]
        fn test_read(
            value in any::<String>(),
        ) {
            let rw_lock = RwLock::new(value.clone());

            let guard = rw_lock.read();
            prop_assert_eq!(guard.deref(), &value);
        }

        #[test]
        fn test_read_waits_for_write(
            value_1 in any::<String>(),
            value_2 in any::<String>(),
        ) {
            let rw_lock = Arc::new(RwLock::new(value_1.clone()));

            let rw_lock_clone = rw_lock.clone();

            let mut guard = rw_lock.write();

            let value_2_clone = value_2.clone();
            let h = thread::spawn(move || {
                let guard = rw_lock_clone.read();
                assert_eq!(guard.deref(), &value_2_clone);
            });

            *guard = value_2;
            drop(guard);

            h.join().unwrap();
        }

        #[test]
        fn test_write_waits_for_read(
            value_1 in any::<String>(),
            value_2 in any::<String>(),
        ) {
            let rw_lock = Arc::new(RwLock::new(value_1.clone()));

            let rw_lock_clone = rw_lock.clone();

            let guard = rw_lock.read();

            let value_2_clone = value_2.clone();
            let h = thread::spawn(move || {
                let mut guard = rw_lock_clone.write();
                *guard = value_2_clone;
            });

            drop(guard);

            h.join().unwrap();

            let guard = rw_lock.read();
            prop_assert_eq!(guard.deref(), &value_2);
        }

        #[test]
        fn test_multiple_read(
            value in any::<String>(),
            threads in 2..100usize,
        ) {
            let rw_lock = Arc::new(RwLock::new(value.clone()));
            let barrier = Arc::new(Barrier::new(threads));

            thread::scope(|s| {
                for _ in 0..threads {
                    let rw_lock = rw_lock.clone();
                    let value = value.clone();
                    let barrier = barrier.clone();

                    s.spawn(move || {
                        let guard = rw_lock.read();

                        // Wait for all threads to acquire the read lock
                        barrier.wait();

                        assert_eq!(*guard, value);
                    });
                }
            });
        }

        #[test]
        fn test_wating_writer_blocks_new_readers(
            value_1 in any::<String>(),
            value_2 in any::<String>(),
        ) {
            let rw_lock = Arc::new(RwLock::new(value_1.clone()));

            // Acquire a read lock
            let read_guard_1 = rw_lock.read();

            let rw_lock_clone = rw_lock.clone();
            let value_2_clone = value_2.clone();

            let (tx, rx) = channel();

            let h_1 = thread::spawn(move || {
                // Acquire a write lock
                let mut write_guard = rw_lock_clone.write();

                // Notify the second thread that the write lock has been acquired
                tx.send(()).unwrap();

                assert_eq!(*write_guard, value_1);
                *write_guard = value_2_clone;
            });

            let rw_lock_clone = rw_lock.clone();
            let value_2_clone = value_2.clone();

            let h_2 = thread::spawn(move || {
                // Wait for the first thread to acquire the write lock
                rx.recv().unwrap();

                // Acquire a read lock
                let read_guard_2 = rw_lock_clone.read();

                // The read lock should have the new value
                assert_eq!(*read_guard_2, value_2_clone);
            });

            // Drop the first read lock, which will allow the write lock to be acquired
            drop(read_guard_1);

            h_1.join().unwrap();
            h_2.join().unwrap();
        }

        #[test]
        fn test_lock_correct_value(
            value in 1..10_000usize,
            threads in 1..10usize,
        ) {
            let rw_lock = Arc::new(RwLock::new(0));

            thread::scope(|s| {
                for _ in 0..threads {
                    let rw_lock = rw_lock.clone();

                    s.spawn(move || {
                        for _ in 0..value {
                            *rw_lock.write() += 1;
                        }
                    });
                }
            });

            let guard = rw_lock.read();
            prop_assert_eq!(*guard, value * threads);
        }
    }
}
