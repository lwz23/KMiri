//! Minimal allocator diagnostic: drop one four-byte Box.
//!
//! This involves neither RCU nor interrupts. It checks whether the freelist
//! write in `SlabSlotList::<8>::push` is independently observable when a
//! 4-byte `Box` is deallocated. Run once under Stacked Borrows and Tree
//! Borrows; this is a classification experiment, not a Step B1 verdict.
#![no_std]
#![deny(unsafe_code)]
#![feature(format_args_nl)]

extern crate alloc;

use alloc::boxed::Box;

#[ostd::ktest::miri_main]
fn kernel_main() {
    drop(Box::new(0u32));
    ostd::miri_println!("alloc-4byte-box-drop: done");
}
