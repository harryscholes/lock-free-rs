use crate::arc::Arc;
use crate::queue::Queue;
use std::{
    hint,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let channel = Arc::new(Channel {
        queue: Queue::new(),
        sender_count: AtomicUsize::new(1),
        receiver_count: AtomicUsize::new(1),
    });
    (
        Sender {
            channel: channel.clone(),
        },
        Receiver { channel },
    )
}

struct Channel<T> {
    queue: Queue<T>,
    sender_count: AtomicUsize,
    receiver_count: AtomicUsize,
}

pub struct Sender<T> {
    channel: Arc<Channel<T>>,
}

impl<T> Sender<T> {
    pub fn send(&self, data: T) -> Result<(), SendError> {
        if self.channel.receiver_count.load(Relaxed) == 0 {
            Err(SendError)
        } else {
            self.channel.queue.enqueue(data);
            Ok(())
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.channel.sender_count.fetch_add(1, Relaxed);

        Self {
            channel: self.channel.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.channel.sender_count.fetch_sub(1, Relaxed);
    }
}

unsafe impl<T> Send for Sender<T> {}

pub struct Receiver<T> {
    channel: Arc<Channel<T>>,
}

impl<T> Receiver<T> {
    pub fn receive(&self) -> Option<T> {
        loop {
            match self.channel.queue.dequeue() {
                Some(item) => return Some(item),
                None if self.channel.sender_count.load(Relaxed) == 0 => return None,
                _ => {
                    hint::spin_loop();
                }
            }
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.channel.receiver_count.fetch_add(1, Relaxed);

        Self {
            channel: self.channel.clone(),
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.channel.receiver_count.fetch_sub(1, Relaxed);
    }
}

unsafe impl<T> Send for Receiver<T> {}

#[derive(Debug)]
pub struct SendError;

#[derive(Debug)]
pub struct ReceiveError;

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use proptest::collection::vec;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_send_receive(data in vec(any::<String>(), 2..10)) {
            let (tx, rx) = channel();

            for item in data.clone() {
                tx.send(item).unwrap();
            }

            drop(tx);

            let mut res = vec![];
            while let Some(item) = rx.receive() {
                res.push(item);
            }

            assert_eq!(res, data);
        }

        #[test]
        fn test_send_receive_thread_safe(item in any::<String>()) {
            let (tx, rx) = channel();

            {
                let item = item.clone();

                thread::spawn(move || {
                    thread::sleep(std::time::Duration::from_millis(1));

                    tx.send(item).unwrap();
                })
                .join()
                .unwrap();
            }

            prop_assert_eq!(rx.receive(), Some(item));
            prop_assert_eq!(rx.receive(), None);
        }

        #[test]
        fn test_multi_producer(mut data in vec(any::<String>(), 1..10)) {
            let (tx, rx) = channel();

            thread::scope(|s| {
                for item in data.clone() {
                    let tx = tx.clone();

                    s.spawn(move || {
                        tx.send(item).unwrap();
                    });
                }

                drop(tx);
            });

            let mut res = vec![];
            while let Some(item) = rx.receive() {
                res.push(item);
            }

            data.sort();
            res.sort();
            prop_assert_eq!(res, data);
        }

        #[test]
        fn test_multi_consumer(mut data in vec(any::<String>(), 1..10)) {
            let (tx, rx) = channel();
            let (tx_2, rx_2) = channel();

            for item in data.clone() {
                tx.send(item).unwrap();
            }

            drop(tx);

            thread::scope(|s| {
                for _ in 0..data.len() {
                    let rx = rx.clone();
                    let tx_2 = tx_2.clone();

                    s.spawn(move || {
                        let item = rx.receive().unwrap();
                        tx_2.send(item).unwrap();
                    });
                }
            });

            drop(tx_2);

            let mut res = vec![];
            while let Some(item) = rx_2.receive() {
                res.push(item);
            }

            data.sort();
            res.sort();
            prop_assert_eq!(res, data);
        }
    }

    #[test]
    fn test_send_error() {
        let (tx, rx) = channel::<()>();
        drop(rx);
        assert!(matches!(tx.send(()), Err(SendError)));
    }

    #[test]
    fn test_receive_none() {
        let (tx, rx) = channel::<()>();
        drop(tx);
        assert_eq!(rx.receive(), None);
    }
}
