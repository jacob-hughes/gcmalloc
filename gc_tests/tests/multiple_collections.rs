// Run-time:
//  status: success

extern crate gcmalloc;

use gcmalloc::{collect, gc::DebugFlags, Debug, Gc};

fn main() {
    let y = Gc::new(456 as usize);
    collect();
    assert!(Debug::is_black(y.as_ptr() as *mut u8));

    // Enable only the preparation phase, to see if it clears y's mark-bit.
    gcmalloc::debug_flags(DebugFlags::new().mark_phase(false).sweep_phase(false));
    collect();
    assert!(!Debug::is_black(y.as_ptr() as *mut u8));

    // Now do a full collection...
    gcmalloc::debug_flags(DebugFlags::new());
    collect();
    // ... and y should still be reachable.
    assert!(Debug::is_black(y.as_ptr() as *mut u8));
}
