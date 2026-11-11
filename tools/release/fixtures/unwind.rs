//! Check the cross-toolchain's unwinder on the minimum supported runtime.
#![forbid(unsafe_code)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Dropped(Arc<AtomicUsize>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn panic_with_drop(count: Arc<AtomicUsize>) {
    let _guard = Dropped(count);
    panic!("unwind probe");
}

fn main() {
    let count = Arc::new(AtomicUsize::new(0));
    let main_count = count.clone();
    assert!(std::panic::catch_unwind(|| panic_with_drop(main_count)).is_err());
    let worker_count = count.clone();
    assert!(
        std::thread::spawn(|| panic_with_drop(worker_count))
            .join()
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    println!("Main and worker panic unwinding ran both destructors");
}
