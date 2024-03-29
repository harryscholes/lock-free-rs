use crate::arc::Arc;
use std::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{
        AtomicBool,
        Ordering::{Acquire, Release},
    },
};

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let channel = Arc::new(Channel {
        data: UnsafeCell::new(MaybeUninit::uninit()),
        ready: AtomicBool::new(false),
    });
    (
        Sender {
            channel: channel.clone(),
        },
        Receiver { channel },
    )
}

struct Channel<T> {
    data: UnsafeCell<MaybeUninit<T>>,
    ready: AtomicBool,
}

pub struct Sender<T> {
    channel: Arc<Channel<T>>,
}

impl<T> Sender<T> {
    pub fn send(self, data: T) {
        unsafe {
            let ptr = self.channel.data.get();
            (*ptr).write(data);
        }
        self.channel.ready.store(true, Release);
    }
}

unsafe impl<T> Send for Sender<T> {}

pub struct Receiver<T> {
    channel: Arc<Channel<T>>,
}

impl<T> Receiver<T> {
    pub fn receive(&self) -> Option<T> {
        if self.channel.ready.swap(false, Acquire) {
            let ptr = self.channel.data.get();
            let data = unsafe { (*ptr).assume_init_read() };
            Some(data)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_send_receive(data in any::<String>()) {
            let (tx, rx) = channel();

            prop_assert_eq!(rx.receive(), None);

            tx.send(data.clone());

            prop_assert_eq!(rx.receive(), Some(data));
            prop_assert_eq!(rx.receive(), None);
        }

        #[test]
        fn test_send_receive_thread_safe(data in any::<String>()) {
            let (tx, rx) = channel();

            {
                let data = data.clone();

                thread::spawn(move || {
                    tx.send(data);
                })
                .join()
                .unwrap();
            }

            prop_assert_eq!(rx.receive(), Some(data));
            prop_assert_eq!(rx.receive(), None);
        }
    }
}
