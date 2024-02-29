use std::{
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{
        fence, AtomicUsize,
        Ordering::{Acquire, Relaxed, Release},
    },
};

#[derive(Debug, PartialEq)]
pub struct Arc<T> {
    inner: NonNull<Inner<T>>,
}

struct Inner<T> {
    data: T,
    count: AtomicUsize,
}

impl<T> Arc<T> {
    pub fn new(data: T) -> Self {
        let inner = NonNull::from(Box::leak(Box::new(Inner {
            data,
            count: AtomicUsize::new(1),
        })));

        Self { inner }
    }

    pub fn try_unwrap(this: Arc<T>) -> Result<T, Arc<T>> {
        // When only one reference exists, we release ownership of the data.
        if this
            .inner()
            .count
            .compare_exchange(1, 0, Release, Relaxed)
            .is_ok()
        {
            // When no more references exist, we use a fence to acquire ownership of the data, so that we can drop it
            // without any other thread accessing it.
            fence(Acquire);

            let inner = unsafe { Box::from_raw(this.inner.as_ptr()) };
            Ok(inner.data)
        } else {
            Err(this)
        }
    }

    pub fn into_inner(this: Arc<T>) -> T {
        match Self::try_unwrap(this) {
            Ok(data) => data,
            Err(_) => panic!("unwrapping failed because more than one reference exists"),
        }
    }

    fn inner(&self) -> &Inner<T> {
        unsafe { self.inner.as_ref() }
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        // Use relaxed ordering because the order of this relative to other operations doesn't matter.
        self.inner().count.fetch_add(1, Relaxed);

        Self { inner: self.inner }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        // When more than one reference exists, we release ownership of the data.
        if self.inner().count.fetch_sub(1, Release) == 1 {
            // When only one reference exists, we acquire ownership of the data so that we can drop it.
            fence(Acquire);

            unsafe { drop(Box::from_raw(self.inner.as_ptr())) };
        }
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner().data
    }
}

unsafe impl<T> Send for Arc<T> {}
unsafe impl<T> Sync for Arc<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{hint, sync, thread};

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1_000))]

        #[test]
        fn test_clone(data in any::<String>()) {
            let a = Arc::new(data);
            prop_assert_eq!(unsafe { a.inner.as_ref().count.load(Relaxed) }, 1);

            let b = a.clone();
            prop_assert_eq!(unsafe { a.inner.as_ref().count.load(Relaxed) }, 2);
            prop_assert_eq!(unsafe { b.inner.as_ref().count.load(Relaxed) }, 2);

            let c = a.clone();
            prop_assert_eq!(unsafe { a.inner.as_ref().count.load(Relaxed) }, 3);
            prop_assert_eq!(unsafe { b.inner.as_ref().count.load(Relaxed) }, 3);
            prop_assert_eq!(unsafe { c.inner.as_ref().count.load(Relaxed) }, 3);

            prop_assert_eq!(&a, &b);
            prop_assert_eq!(b, c);
        }

        #[test]
        fn test_drop(data in any::<String>()) {
            let a = Arc::new(data);
            let b = a.clone();
            let c = a.clone();

            prop_assert_eq!(unsafe { a.inner.as_ref().count.load(Relaxed) }, 3);
            prop_assert_eq!(unsafe { b.inner.as_ref().count.load(Relaxed) }, 3);
            prop_assert_eq!(unsafe { c.inner.as_ref().count.load(Relaxed) }, 3);

            drop(a);
            prop_assert_eq!(unsafe { b.inner.as_ref().count.load(Relaxed) }, 2);
            prop_assert_eq!(unsafe { c.inner.as_ref().count.load(Relaxed) }, 2);

            drop(b);
            prop_assert_eq!(unsafe { c.inner.as_ref().count.load(Relaxed) }, 1);
        }


        #[test]
        fn test_send(data in any::<String>()) {
            let a = Arc::new(data);
            let b = a.clone();
            let b = thread::spawn(move || b).join().unwrap();
            prop_assert_eq!(a, b);
        }

        #[test]
        fn test_deref(data in any::<String>()) {
            let a = Arc::new(data.clone());
            prop_assert_eq!(&*a, &data);

            let a = Arc::new(());
            prop_assert_eq!(*a, ());
        }

        #[test]
        fn test_try_unwrap(data in any::<String>()) {
            let a = Arc::new(data.clone());
            let b = a.clone();
            let a = Arc::try_unwrap(a).unwrap_err();
            let b = Arc::try_unwrap(b).unwrap_err();
            drop(a);
            prop_assert_eq!(Arc::try_unwrap(b), Ok(data));
        }

        #[test]
        fn test_try_unwrap_multiple_threads(data in any::<String>()) {
            let a = Arc::new(data.clone());
            let b = a.clone();

            let (tx, rx) = sync::mpsc::channel();

            let h = thread::spawn(move|| {
                let mut b = Arc::try_unwrap(b).unwrap_err();

                // Signal that we've tried, and failed, to unwrap the data.
                tx.send(()).unwrap();

                loop {
                    // This will fail until `a` is dropped by the other thread.
                    match Arc::try_unwrap(b) {
                        Ok(d) => return d,
                        Err(a) => b = a,
                    }

                    hint::spin_loop();
                }
            });

            // Once the other thread has tried, and failed, to unwrap the data, `a` is dropped so that the data can be unwrapped.
            rx.recv().unwrap();
            drop(a);

            let d = h.join().unwrap();
            prop_assert_eq!(d, data);
        }
    }
}
