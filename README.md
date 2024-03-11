# lock-free-rs
Lock-free stuff in Rust

- `Arc`: an atomically reference-counted smart pointer
- `mpmc::channel`: a multi-producer multi-consumer channel
- `mpsc::channel`: a multi-producer single-consumer channel
- `oneshot::channel`: a channel for sending a single value across threads
- `Queue`: a Michael & Scott queue
- `SpinLock`: a spin lock