use std::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
    thread,
    time::Duration,
};

const INITIAL_BACKOFF: Duration = Duration::from_nanos(1);
const MAX_BACKOFF: Duration = Duration::from_nanos(100);

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
        let mut backoff = INITIAL_BACKOFF;

        let new_node = Node::new(item);
        let new_node_ptr = Box::into_raw(Box::new(new_node));

        loop {
            let tail_ptr = self.tail.load(Ordering::Acquire);
            let tail_ref = unsafe { &*tail_ptr };
            let next_ptr = tail_ref.next.load(Ordering::Acquire);

            if next_ptr.is_null() {
                // The tail is pointing at the last node, so try to insert the new node after it.
                // This can fail if other threads are concurrently enqueuing,
                // but the queue will remain in a consistent state.
                if tail_ref
                    .next
                    .compare_exchange(next_ptr, new_node_ptr, Ordering::Release, Ordering::Relaxed)
                    .is_ok()
                {
                    // The new node has been inserted at the end of the queue, so try to move the tail to point to it.
                    // This can fail if another thread has already updated the tail, but the queue will remain in a
                    // consistent state.
                    _ = self.tail.compare_exchange(
                        tail_ptr,
                        new_node_ptr,
                        Ordering::Release,
                        Ordering::Relaxed,
                    );

                    return;
                }

                thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
            } else {
                // The tail isn't pointing to the last node in the queue, so try to swing the tail to the next node
                // and retry. This can fail if other threads are concurrently enqueuing, but the queue will remain
                // in a consistent state.
                _ = self.tail.compare_exchange(
                    tail_ptr,
                    next_ptr,
                    Ordering::Release,
                    Ordering::Relaxed,
                );
            }
        }
    }

    pub fn dequeue(&self) -> Option<T> {
        let mut backoff = Duration::from_millis(10);

        loop {
            let head_ptr = self.head.load(Ordering::Acquire);
            let head_ref = unsafe { &*head_ptr };
            let next_ptr = head_ref.next.load(Ordering::Acquire);

            if next_ptr.is_null() {
                // The queue is empty.
                return None;
            }

            // The queue is not empty.
            // Try to dequeue the empty head node by swinging the head to point at the next node.
            if self
                .head
                .compare_exchange(head_ptr, next_ptr, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                // The empty head node has been dequeued and the next node is now at the head.
                // Take the item out of the new head node.
                let next_ref = unsafe { &mut *next_ptr };
                let item = next_ref.item.take();

                // TODO Deallocate the node's memory.
                // Epoch-based memory reclamation e.g. crossbeam-epoch could be used to safely deallocate the node's memory.
                // To minimise the amount of memory leaked, we take the item out of its `Option`, leaving `None` behind.

                return item;
            }

            thread::sleep(backoff);
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    pub fn is_empty(&self) -> bool {
        let head_ptr = self.head.load(Ordering::Relaxed);
        let head_ref = unsafe { &*head_ptr };
        let next_ptr = head_ref.next.load(Ordering::Relaxed);

        next_ptr.is_null()
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

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

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{atomic::AtomicBool, Arc},
        thread,
    };

    use super::*;
    use proptest::{collection::vec, prelude::*};

    proptest! {
        #[test]
        fn test_pointers(
            items in vec(any::<usize>(), 3)
        ) {
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

            q.enqueue(items[0]);
            let head_2 = q.head.load(Ordering::Relaxed);
            let tail_2 = q.tail.load(Ordering::Relaxed);
            assert!(!ptr::eq(head_2, tail_2));
            assert!(ptr::eq(head_2, head_0));

            q.enqueue(items[1]);
            let head_3 = q.head.load(Ordering::Relaxed);
            let tail_3 = q.tail.load(Ordering::Relaxed);
            assert!(!ptr::eq(head_3, tail_3));
            assert!(ptr::eq(head_3, head_0));

            q.enqueue(items[2]);
            let head_4 = q.head.load(Ordering::Relaxed);
            let tail_4 = q.tail.load(Ordering::Relaxed);
            assert!(!ptr::eq(head_4, tail_4));
            assert!(ptr::eq(head_4, head_0));

            assert_eq!(q.dequeue(), Some(items[0]));
            let head_5 = q.head.load(Ordering::Relaxed);
            let tail_5 = q.tail.load(Ordering::Relaxed);
            assert!(ptr::eq(head_5, tail_2));
            assert!(ptr::eq(tail_5, tail_4));

            assert_eq!(q.dequeue(), Some(items[1]));
            let head_6 = q.head.load(Ordering::Relaxed);
            let tail_6 = q.tail.load(Ordering::Relaxed);
            assert!(ptr::eq(head_6, tail_3));
            assert!(ptr::eq(tail_6, tail_4));

            assert_eq!(q.dequeue(), Some(items[2]));
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
        fn test_fill_and_empty_repeatedly(
            test_size in 1..1_000usize,
            num_repeats in 1..10usize
        ) {
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
        fn test_multiple_concurrent_enqueuers_single_dequeuer(
            num_threads in 2..100usize,
            items in vec(any::<usize>(), 1..1_000)
        ) {
            let mut expected: HashMap<usize, usize> = HashMap::new();
            for item in items.clone() {
                *expected.entry(item).or_default() += num_threads;
            }

            let q = Arc::new(Queue::new());

            thread::scope(|s| {
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
        fn test_multiple_concurrent_enqueuers_single_concurrent_dequeuer(
            num_threads in 2..100usize,
            items in vec(any::<usize>(), 1..1_000)
        ) {
            let mut expected: HashMap<usize, usize> = HashMap::new();
            for item in items.clone() {
                *expected.entry(item).or_default() += num_threads;
            }

            let q = Arc::new(Queue::new());

            let (tx, rx) = std::sync::mpsc::channel();

            for _ in 0..num_threads {
                let items = items.clone();
                let q = q.clone();
                let tx = tx.clone();

                thread::spawn(move || {
                    for item in items {
                        q.enqueue(item);
                    }

                    tx.send(()).unwrap();
                });
            }

            // Once all enqueuers have finished, all references to `tx` will have been dropped.
            drop(tx);

            // Keep dequeuing until all enqueuers have finished and the queue is empty.
            while rx.recv().is_ok() {
                while let Some(item) = q.dequeue() {
                    *expected.entry(item).or_default() -= 1;
                }
            }

            expected.retain(|_, v| *v > 0);

            prop_assert!(expected.is_empty());
        }

        #[test]
        fn test_single_concurrent_enqueuer_multiple_concurrent_dequeuers(
            num_threads in 2..100usize,
            mut items in vec(any::<usize>(), 1..1_000)
        ) {
            let q = Arc::new(Queue::new());

            let (tx, rx) = std::sync::mpsc::channel();
            let enqueuing = Arc::new(AtomicBool::new(true));

            for _ in 0..num_threads {
                let q = q.clone();
                let tx = tx.clone();
                let enqueuing = enqueuing.clone();

                thread::spawn(move || {
                    loop {
                        while let Some(item) = q.dequeue() {
                            tx.send(item).unwrap();
                        }

                        if !enqueuing.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                });
            }

            drop(tx);

            for item in items.clone() {
                q.enqueue(item);
            }

            enqueuing.store(false, Ordering::Relaxed);

            let mut res = vec![];
            while let Ok(item) = rx.recv() {
                res.push(item);
            }

            items.sort();
            res.sort();

            prop_assert_eq!(res, items);
        }

        #[test]
        fn test_multiple_concurrent_enqueuers_multiple_concurrent_dequeuers(
            num_threads in 2..100usize,
            items in vec(any::<usize>(), 1..1_000)
        ) {
            let mut expected: HashMap<usize, usize> = HashMap::new();
            for item in items.clone() {
                *expected.entry(item).or_default() += num_threads;
            }

            let q = Arc::new(Queue::new());

            let (enqueuer_tx, enqueuer_rx) = std::sync::mpsc::channel();
            let (dequeuer_tx, dequeuer_rx) = std::sync::mpsc::channel();
            let enqueuing = Arc::new(AtomicBool::new(true));

            for _ in 0..num_threads {
                let items = items.clone();
                let q = q.clone();
                let enqueuer_tx = enqueuer_tx.clone();

                thread::spawn(move || {
                    for item in items {
                        q.enqueue(item);
                    }

                    enqueuer_tx.send(()).unwrap();
                });
            }

            // Once all enqueuers have finished, all references to `enqueuer_tx` will have been dropped.
            drop(enqueuer_tx);

            for _ in 0..num_threads {
                let q = q.clone();
                let dequeuer_tx = dequeuer_tx.clone();
                let enqueuing = enqueuing.clone();

                thread::spawn(move || {
                    loop {
                        while let Some(item) = q.dequeue() {
                            dequeuer_tx.send(item).unwrap();
                        }

                        if !enqueuing.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                });
            }
            drop(dequeuer_tx);

            // Wait until all enqueuers have finished.
            loop {
                if enqueuer_rx.recv().is_err() {
                    enqueuing.store(false, Ordering::Relaxed);
                    break;
                }
            }

            // Keep dequeuing until all enqueuers have finished and the queue is empty.
            while let Ok(item) = dequeuer_rx.recv() {
                *expected.get_mut(&item).unwrap() -= 1;
            }

            expected.retain(|_, v| *v > 0);

            prop_assert!(expected.is_empty());
        }

        #[test]
        fn test_is_empty(test_size in 1..1_000usize) {
            let q: Queue<usize> = Queue::new();
            assert!(q.is_empty());

            for item in 0..test_size {
                q.enqueue(item);
                assert!(!q.is_empty());
            }

            for _ in 0..test_size {
                assert!(!q.is_empty());
                q.dequeue();
            }

            assert!(q.is_empty());
        }
    }
}
