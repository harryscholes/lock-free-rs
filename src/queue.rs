use std::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
    time::Duration,
};

#[derive(Debug)]
struct Node<T> {
    item: Option<T>,
    next: AtomicPtr<Node<T>>,
}

impl<T> Node<T> {
    fn new(item: T) -> Self {
        Self {
            item: Some(item),
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }

    fn null() -> Self {
        Self {
            item: None,
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }
}

#[derive(Debug)]
pub struct Queue<T> {
    head: AtomicPtr<Node<T>>,
    tail: AtomicPtr<Node<T>>,
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        let sentinel = Box::into_raw(Box::new(Node::null()));
        Self {
            head: AtomicPtr::new(sentinel),
            tail: AtomicPtr::new(sentinel),
        }
    }

    pub fn enqueue(&self, item: T) {
        let mut backoff = Duration::from_millis(10);

        let new_node = Node::new(item);
        let new_node_ptr = Box::into_raw(Box::new(new_node));

        loop {
            let tail_ptr = self.tail.load(Ordering::SeqCst);
            let tail_ref = unsafe { &*tail_ptr };
            let next_ptr = tail_ref.next.load(Ordering::SeqCst);

            if next_ptr.is_null() {
                // The tail is pointing at the last node, so try to insert the new node after it.
                // This can fail if other threads are concurrently enqueuing,
                // but the queue will remain in a consistent state.
                if tail_ref
                    .next
                    .compare_exchange(next_ptr, new_node_ptr, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    // The new node has been inserted at the end of the queue, so try to move the tail to point to it.
                    // This can fail if another thread has already updated the tail, but the queue will remain in a
                    // consistent state.
                    _ = self.tail.compare_exchange(
                        tail_ptr,
                        new_node_ptr,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    );

                    return;
                }
            } else {
                // The tail isn't pointing to the last node in the queue, so try to swing the tail to the next node
                // and retry. This can fail if other threads are concurrently enqueuing, but the queue will remain
                // in a consistent state.
                _ = self.tail.compare_exchange(
                    tail_ptr,
                    next_ptr,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
            }

            std::thread::sleep(backoff);
            backoff *= 2;
        }
    }

    pub fn dequeue(&self) -> Option<T> {
        let mut backoff = Duration::from_millis(10);

        loop {
            let head_ptr = self.head.load(Ordering::SeqCst);
            let head_ref = unsafe { &*head_ptr };
            let next_ptr = head_ref.next.load(Ordering::SeqCst);

            if next_ptr.is_null() {
                // The queue is empty.
                return None;
            }

            // The queue is not empty.
            // Try to dequeue the empty head node by swinging the head to point at the next node.
            if self
                .head
                .compare_exchange(head_ptr, next_ptr, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                // The empty head node has been dequeued and the next node is now at the head.
                // Take the item out of the new head node.
                let next_ref = unsafe { &mut *next_ptr };
                // let item = next_ref.item.take();
                let next_node = unsafe { ptr::read(next_ref as *const Node<T>) };

                // TODO Deallocate the node's memory using epoch-based memory reclamation e.g. crossbeam-epoch.

                // Take ownership of the dequeued node, so it will get deallocated when it gets dropped.
                // _ = unsafe { Box::from_raw(head_ptr) };

                // dbg!(&next_node.item);

                return next_node.item;
            }

            std::thread::sleep(backoff);
            backoff *= 2;
        }
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use super::*;

    #[test]
    fn test_pointers() {
        let q: Queue<usize> = Queue::new();
        let head_0 = q.head.load(Ordering::Relaxed);
        let tail_0 = q.tail.load(Ordering::Relaxed);
        assert!(ptr::eq(head_0, tail_0));

        assert_eq!(q.dequeue(), None);
        let head_1 = q.head.load(Ordering::Relaxed);
        let tail_1 = q.tail.load(Ordering::Relaxed);
        assert!(ptr::eq(head_1, tail_1));
        assert!(ptr::eq(head_1, head_0));
        assert!(ptr::eq(tail_1, tail_0));

        q.enqueue(0);
        let head_2 = q.head.load(Ordering::Relaxed);
        let tail_2 = q.tail.load(Ordering::Relaxed);
        assert!(!ptr::eq(head_2, tail_2));
        assert!(ptr::eq(head_2, head_0));

        q.enqueue(1);
        let head_3 = q.head.load(Ordering::Relaxed);
        let tail_3 = q.tail.load(Ordering::Relaxed);
        assert!(!ptr::eq(head_3, tail_3));
        assert!(ptr::eq(head_3, head_0));

        q.enqueue(2);
        let head_4 = q.head.load(Ordering::Relaxed);
        let tail_4 = q.tail.load(Ordering::Relaxed);
        assert!(!ptr::eq(head_4, tail_4));
        assert!(ptr::eq(head_4, head_0));

        assert_eq!(q.dequeue(), Some(0));
        let head_5 = q.head.load(Ordering::Relaxed);
        let tail_5 = q.tail.load(Ordering::Relaxed);
        assert!(ptr::eq(head_5, tail_2));
        assert!(ptr::eq(tail_5, tail_4));

        assert_eq!(q.dequeue(), Some(1));
        let head_6 = q.head.load(Ordering::Relaxed);
        let tail_6 = q.tail.load(Ordering::Relaxed);
        assert!(ptr::eq(head_6, tail_3));
        assert!(ptr::eq(tail_6, tail_4));

        assert_eq!(q.dequeue(), Some(2));
        let head_7 = q.head.load(Ordering::Relaxed);
        let tail_7 = q.tail.load(Ordering::Relaxed);
        assert!(ptr::eq(head_7, tail_4));
        assert!(ptr::eq(tail_7, tail_4));

        assert_eq!(q.dequeue(), None);
        let head_8 = q.head.load(Ordering::Relaxed);
        let tail_8 = q.tail.load(Ordering::Relaxed);
        assert!(ptr::eq(head_8, tail_4));
        assert!(ptr::eq(tail_8, tail_4));
    }

    #[test]
    fn test_fill_and_empty() {
        let test_size: usize = 100_000;

        let q = Queue::new();

        for item in 0..test_size {
            q.enqueue(item);
        }

        let mut dst = vec![];

        while let Some(item) = q.dequeue() {
            dst.push(item);
        }

        assert_eq!(dst, Vec::from_iter(0..test_size));
    }

    #[test]
    fn test_fill_and_empty_repeatedly() {
        let test_size: usize = 10_000;
        let num_repeats: usize = 100;

        let q = Queue::new();

        for _ in 0..num_repeats {
            for item in 0..test_size {
                q.enqueue(item);
            }

            // Don't force drain the queue.
            for _ in 0..test_size {
                q.dequeue();
            }
        }

        for item in 0..test_size {
            q.enqueue(item);
        }

        let mut res = vec![];

        // Drain the queue and check that all items are present, once, and in order.
        while let Some(item) = q.dequeue() {
            res.push(item);
        }

        assert_eq!(res, Vec::from_iter(0..test_size));
    }

    #[test]
    fn test_enqueue_concurrently() {
        let num_threads: usize = 10;
        let test_size: usize = 10_000;

        let q = Arc::new(Queue::new());

        std::thread::scope(|s| {
            for t in 0..num_threads {
                let q = q.clone();
                s.spawn(move || {
                    for i in t * test_size..(t + 1) * test_size {
                        q.enqueue(i);
                    }
                });
            }
        });

        let mut res = vec![];

        while let Some(item) = q.dequeue() {
            res.push(item);
        }

        res.sort();

        assert_eq!(res, Vec::from_iter(0..num_threads * test_size));
    }

    #[test]
    fn test_enqueue_and_dequeue_concurrently() {
        let num_threads: usize = 10;
        let test_size: usize = 10_000;

        let expected = HashSet::from_iter(0..num_threads * test_size);

        let q = Arc::new(Queue::new());

        for t in 0..num_threads {
            let q = q.clone();

            std::thread::spawn(move || {
                for item in t * test_size..(t + 1) * test_size {
                    q.enqueue(item);
                }
            });
        }

        let mut res = HashSet::new();

        while res != expected {
            while let Some(item) = q.dequeue() {
                res.insert(item);
            }
        }

        assert_eq!(res, expected);
    }
}

#[cfg(test)]
mod proptests {
    use std::{
        collections::HashMap,
        sync::{atomic::AtomicBool, Arc},
    };

    use super::*;
    use proptest::{collection::vec, prelude::*};

    proptest! {
        #[test]
        fn proptest_multiple_concurrent_enqueuers_single_dequeuer(
            num_threads in 1..10usize,
            items in vec(any::<usize>(), 1..1_000)
        ) {
            let mut expected: HashMap<usize, usize> = HashMap::new();
            for item in items.clone() {
                *expected.entry(item).or_default() += num_threads;
            }

            let q = Arc::new(Queue::new());

            std::thread::scope(|s| {
                for _ in 0..num_threads {
                    let items = items.clone();
                    let q = q.clone();

                    s.spawn(move || {
                        for item in items {
                            q.enqueue(item);
                        }
                    });
                }
            });

            while let Some(item) = q.dequeue() {
                *expected.entry(item).or_default() -= 1;
            }

            expected.retain(|_, v| *v > 0);

            prop_assert!(expected.is_empty());
        }

        #[test]
        fn proptest_multiple_concurrent_enqueuers_single_concurrent_dequeuer(
            num_threads in 1..10usize,
            items in vec(any::<usize>(), 1..1_000)
        ) {
            let mut expected: HashMap<usize, usize> = HashMap::new();
            for item in items.clone() {
                *expected.entry(item).or_default() += num_threads;
            }

            let q = Arc::new(Queue::new());

            for _ in 0..num_threads {
                let items = items.clone();
                let q = q.clone();

                std::thread::spawn(move || {
                    for item in items {
                        q.enqueue(item);
                    }
                });
            }

            while !expected.is_empty() {
                while let Some(item) = q.dequeue() {
                    *expected.entry(item).or_default() -= 1;
                }
                expected.retain(|_, v| *v > 0);
            }

            prop_assert!(expected.is_empty());
        }

        #[test]
        fn proptest_multiple_concurrent_enqueuers_multiple_concurrent_dequeuers(
            num_threads in 2..10usize,
            items in vec(any::<usize>(), 1..1_000)
        ) {
            let mut expected: HashMap<usize, usize> = HashMap::new();
            for item in items.clone() {
                *expected.entry(item).or_default() += num_threads;
            }

            let cancelled = Arc::new(AtomicBool::new(false));

            let q = Arc::new(Queue::new());

            for _ in 0..num_threads {
                let q = q.clone();
                let items = items.clone();

                std::thread::spawn(move || {
                    for item in items {
                        q.enqueue(item);
                    }
                });
            }

            let (tx, rx) = std::sync::mpsc::channel();

            for _ in 0..num_threads {
                let q = q.clone();
                let tx = tx.clone();
                let cancelled = cancelled.clone();

                std::thread::spawn(move || {
                    let mut backoff = Duration::from_millis(1);

                    while !cancelled.load(Ordering::SeqCst) {
                        match q.dequeue() {
                            Some(item) => {
                                tx.send(item).unwrap();
                                backoff /= 2;
                            }
                            None => {
                                std::thread::sleep(backoff);
                                backoff *= 2;
                            }
                        }
                    }
                });
            }

            // std::thread::spawn({
            //     let q = q.clone();
            //     let cancelled = cancelled.clone();

            //     move || {
            //         while !cancelled.load(Ordering::SeqCst) {
            //             std::thread::sleep(std::time::Duration::from_millis(2000));

            //             let head_ptr = q.head.load(Ordering::SeqCst);
            //             let tail_ptr = q.tail.load(Ordering::SeqCst);
            //             let next_ptr = unsafe { &*tail_ptr }.next.load(Ordering::SeqCst);
            //             dbg!(head_ptr, tail_ptr, next_ptr);
            //         }
            //     }
            // });

            while let Ok(item) = rx.recv() {
                *expected.get_mut(&item).unwrap() -= 1;

                expected.retain(|_, v| *v > 0);

                if expected.is_empty() {
                    drop(rx);
                    cancelled.store(true, Ordering::SeqCst);
                    break;
                }
            }

            prop_assert!(expected.is_empty());
        }
    }
}
