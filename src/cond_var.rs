use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

use atomic_wait::{wait, wake_all, wake_one};

use crate::mutex::MutexGuard;

pub struct Condvar {
    counter: AtomicU32,
    num_waiters: AtomicUsize,
}

impl Condvar {
    pub const fn new() -> Self {
        Self {
            counter: AtomicU32::new(0),
            num_waiters: AtomicUsize::new(0),
        }
    }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        self.num_waiters.fetch_add(1, Relaxed);

        let count = self.counter.load(Relaxed);

        let mutex = guard.mutex;
        drop(guard);

        wait(&self.counter, count);

        self.num_waiters.fetch_sub(1, Relaxed);

        mutex.lock()
    }

    pub fn notify_one(&self) {
        if self.num_waiters.load(Relaxed) > 0 {
            self.counter.fetch_add(1, Relaxed);
            wake_one(&self.counter);
        }
    }

    pub fn notify_all(&self) {
        if self.num_waiters.load(Relaxed) > 0 {
            self.counter.fetch_add(1, Relaxed);
            wake_all(&self.counter);
        }
    }
}

impl Default for Condvar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{thread, time::Duration};

    use proptest::prelude::*;

    use crate::mutex::Mutex;

    proptest! {
        #[test]
        fn test_notify_one(
            value in any::<usize>()
        ) {
            let mutex = Mutex::new(value);
            let condvar = Condvar::new();

            let mut wakeups = 0;

            thread::scope(|s| {
                s.spawn(|| {
                    let mut m = mutex.lock();
                    while *m == value {
                        m = condvar.wait(m);
                        wakeups += 1;
                    }
                    assert_eq!(*m, value + 1);
                });

                s.spawn(|| {
                    thread::sleep(Duration::from_millis(10));
                    *mutex.lock() += 1;
                    condvar.notify_one();
                });
            });

            prop_assert!(wakeups > 0);
        }

        #[test]
        fn test_notify_all(
            value in any::<usize>()
        ) {
            let mutex = Mutex::new(value);
            let condvar = Condvar::new();

            let wakeups = AtomicUsize::new(0);

            thread::scope(|s| {
                for _ in 0..5 {
                    s.spawn(|| {
                        let mut m = mutex.lock();
                        while *m == value {
                            m = condvar.wait(m);
                            wakeups.fetch_add(1, Relaxed);
                        }
                    });
                }

                thread::sleep(Duration::from_millis(10));
                *mutex.lock() += 1;
                condvar.notify_all();
            });

            prop_assert!(wakeups.load(Relaxed) >= 5);
        }
    }
}
