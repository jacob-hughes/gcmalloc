// Copyright (c) 2019 King's College London created by the Software Development
// Team <http://soft-dev.org/>
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0>, or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, or the UPL-1.0 license
// <http://opensource.org/licenses/UPL> at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::{
    alloc::{AllocMetadata, PtrInfo},
    Gc, GC_ALLOCATOR,
};
use std::{
    alloc::{Alloc, Layout},
    ptr::NonNull,
    sync::Mutex,
};

static WORD_SIZE: usize = std::mem::size_of::<usize>(); // Bytes

type Address = usize;

type Word = usize;

type StackScanCallback = extern "sysv64" fn(&mut Collector, Address);
#[link(name = "SpillRegisters", kind = "static")]
extern "sysv64" {
    // Pass a type-punned pointer to the collector and move it to the asm spill
    // code. This is so it can be passed straight back as the implicit `self`
    // address in the callback.
    #[allow(improper_ctypes)]
    fn spill_registers(collector: *mut u8, callback: StackScanCallback);
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum CollectorState {
    Ready,
    RootScanning,
    Marking,
    Sweeping,
}

pub struct DebugFlags {
    pub mark_phase: bool,
    pub sweep_phase: bool,
}

impl DebugFlags {
    pub fn new() -> Self {
        Self {
            mark_phase: true,
            sweep_phase: true,
        }
    }

    pub fn mark_phase(mut self, val: bool) -> Self {
        self.mark_phase = val;
        self
    }

    pub fn sweep_phase(mut self, val: bool) -> Self {
        self.sweep_phase = val;
        self
    }
}

/// Colour of an object used during marking phase (see Dijkstra tri-colour
/// abstraction)
#[derive(PartialEq, Eq)]
pub(crate) enum Colour {
    Black,
    White,
}

pub(crate) struct Collector {
    worklist: Vec<PtrInfo>,
    black: bool,
    pub(crate) debug_flags: DebugFlags,
    pub(crate) state: Mutex<CollectorState>,
}

impl Collector {
    pub(crate) fn new(debug_flags: DebugFlags) -> Self {
        Self {
            worklist: Vec::new(),
            black: true,
            debug_flags,
            state: Mutex::new(CollectorState::Ready),
        }
    }

    pub(crate) fn current_black(&self) -> bool {
        self.black
    }

    pub(crate) fn collect(&mut self) {
        // First check that no call to collect is active
        {
            let mut cstate = self.state.lock().unwrap();
            match *cstate {
                CollectorState::Ready => *cstate = CollectorState::RootScanning,
                _ => {
                    // The collector is running on another thread.
                    return;
                }
            }
        }

        // Register spilling is platform specific. This is implemented in
        // an assembly stub. The fn to scan the stack is passed as a callback
        unsafe { spill_registers(self as *mut Collector as *mut u8, Collector::scan_stack) }

        if self.debug_flags.mark_phase {
            self.enter_mark_phase();
        }

        if self.debug_flags.sweep_phase {
            self.enter_sweep_phase();
        }

        *self.state.lock().unwrap() = CollectorState::Ready;
    }

    /// The worklist is populated with potential GC roots during the stack
    /// scanning phase. The mark phase then traces through this root-set until
    /// it finds GC objects. Once found, a GC object is coloured black to
    /// indicate that it is reachable by the mutator, and is therefore *not* a
    /// candidate for reclaimation.
    fn enter_mark_phase(&mut self) {
        *self.state.lock().unwrap() = CollectorState::Marking;

        while !self.worklist.is_empty() {
            let PtrInfo { ptr, size, gc } = self.worklist.pop().unwrap();

            if gc {
                // For GC objects, the pointer recorded in the alloc metadata
                // list points to the beginning of the object -- *not* the
                // object's header. This means that unlike regular allocations,
                // `ptr` will never point to the beginning of the allocation
                // block.
                let obj = unsafe { Gc::from_raw(ptr as *const i8) };
                if self.colour(obj) == Colour::Black {
                    continue;
                }
                self.mark(obj, Colour::Black);
            }

            // Check each word in the allocation block for pointers.
            for addr in (ptr..ptr + size).step_by(WORD_SIZE) {
                let word = unsafe { *(addr as *const Word) };

                if let Some(ptrinfo) = AllocMetadata::find(word) {
                    self.worklist.push(ptrinfo)
                }
            }
        }
    }

    fn enter_sweep_phase(&mut self) {
        *self.state.lock().unwrap() = CollectorState::Sweeping;

        for PtrInfo { ptr, .. } in AllocMetadata.iter().filter(|x| x.gc) {
            let obj = unsafe { Gc::from_raw(ptr as *const i8) };
            if self.colour(obj) == Colour::White {
                unsafe {
                    let baseptr = (ptr as *mut u8).sub(obj.base_ptr_offset());
                    GC_ALLOCATOR.dealloc(
                        NonNull::new_unchecked(baseptr as *mut u8),
                        Layout::new::<usize>(),
                    );
                }
            }
        }

        // Flip the meaning of the mark bit, i.e. if false == Black, then it
        // becomes false == white. This is a simplification which allows us to
        // avoid resetting the mark bit for every survived object after
        // collection. Since we do not implement a marking bitmap and instead
        // store this mark bit in each object header, this would be a very
        // expensive operation.
        self.black = !self.black;
    }

    #[no_mangle]
    extern "sysv64" fn scan_stack(&mut self, rsp: Address) {
        let stack_top = unsafe { get_stack_start() }.unwrap();

        for stack_address in (rsp..stack_top).step_by(WORD_SIZE) {
            let stack_word = unsafe { *(stack_address as *const Word) };
            if let Some(ptr_info) = AllocMetadata::find(stack_word) {
                self.worklist.push(ptr_info)
            }
        }
    }

    pub(crate) fn colour(&self, obj: Gc<i8>) -> Colour {
        if obj.mark_bit() == self.black {
            Colour::Black
        } else {
            Colour::White
        }
    }

    fn mark(&self, obj: Gc<i8>, colour: Colour) {
        match colour {
            Colour::Black => obj.set_mark_bit(self.black),
            Colour::White => obj.set_mark_bit(!self.black),
        };
    }
}

/// Attempt to get the starting address of the stack via the pthread API. This
/// is highly platform specific. It is used as the lower bound for the range of
/// on-stack-values which are scanned for potential roots in GC.
#[cfg(target_os = "linux")]
unsafe fn get_stack_start() -> Option<Address> {
    let mut attr: libc::pthread_attr_t = std::mem::zeroed();
    assert_eq!(libc::pthread_attr_init(&mut attr), 0);
    let ptid = libc::pthread_self();
    let e = libc::pthread_getattr_np(ptid, &mut attr);
    if e != 0 {
        assert_eq!(libc::pthread_attr_destroy(&mut attr), 0);
        return None;
    }
    let mut stackaddr = std::ptr::null_mut();
    let mut stacksize = 0;
    assert_eq!(
        libc::pthread_attr_getstack(&attr, &mut stackaddr, &mut stacksize),
        0
    );
    return Some((stackaddr as usize + stacksize) as Address);
}
