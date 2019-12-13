// Run-time:
//  status: success

extern crate gcmalloc;

use gcmalloc::{collect, gc::DebugFlags, Debug, Gc};

fn main() {
    let threshold = 5;
    // Lower the threshold so that the test doesn't take forever.
    gcmalloc::set_threshold(threshold);

    let x = Gc::new("Hello World".to_string());
    assert!(!Debug::is_black(x));

    for i in 0..threshold {
        let x = Gc::new(123 as usize);
    }

    assert!(Debug::is_black(x));
}