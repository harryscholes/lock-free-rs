# concurrent-rs

Concurrency primitives in Rust

- `Arc`: an atomically reference-counted smart pointer
- `CondVar`: a condition variable
- `mpmc::channel`: a multi-producer multi-consumer channel
- `mpsc::channel`: a multi-producer single-consumer channel
- `Mutex`: a mutual exclusion lock
- `oneshot::channel`: a channel for sending a single value across threads
- `Queue`: a Michael & Scott lock-free queue
- `RwLock`: a reader-writer lock
- `SpinLock`: a spin lock
