;; 1 2 3 0 12 0

;; This is a simple Rust program, converted to WASM, lightly modified to fit the API and pasted in
;; I could set up a Cargo project, but it's so tiny I didn't bother
;; I've been just using the Playground to compile it
;; Source code is at the bottom
(module
  (type $t1 (func (param i32) (result i32)))
  (type $t2 (func (param i32 i32)))
  (import "spv" "id" (global $id i32)) ;; I changed it from a function to a global, because Rust doesn't support global imports
  (import "spv" "buffer:0:0:load" (func $buffer:0:0:load (type $t1)))
  (import "spv" "buffer:0:0:store" (func $buffer:0:0:store (type $t2)))
  (func $main (export "main") ;; I also removed the arguments and the result
    (local $l2 i32) (local $l3 i32)
    (local.set $l2
      (i32.const 0))
    (block $B0
      (br_if $B0
        (i32.gt_u
          (local.tee $l3
            (call $buffer:0:0:load
              (i32.shl
                  (get_global $id) ;; Changed from a call to get_global
                  (i32.const 2))))
          (i32.const 4)))
      (local.set $l2
        (i32.load
          (i32.add
            (i32.shl
              (local.get $l3)
              (i32.const 2))
            (i32.const 1048576)))
            ))
    (call $buffer:0:0:store
      (i32.shl
        (get_global $id) ;; Changed from call $id
        (i32.const 2))
      (local.get $l2))) ;; Removed a return value of 0
  (start $main) ;; I added this too
  (table $T0 1 1 funcref)
  (memory $memory (export "memory") 17)
  (global $__data_end (export "__data_end") i32 (i32.const 1048596))
  (global $__heap_base (export "__heap_base") i32 (i32.const 1048596))
  (data $d0 (i32.const 1048576) "\01\00\00\00\02\00\00\00\03\00\00\00\00\00\00\00\0c\00\00\00"))

;; Original source:
(;
#![feature(start)]
#![no_std]

#[link(wasm_import_module="spv")]
extern {
    fn trap() -> !;
    fn id() -> usize;
}

#[panic_handler]
fn handle_panic(_x: &core::panic::PanicInfo) -> ! {
    unsafe {
        trap()
    }
}

fn thread_id() -> &'static mut u32 {
    unsafe {
        core::mem::transmute(id())
    }
}

fn do_something(val: u32) -> u32 {
    match val {
        0 => 1,
        1 => 2,
        2 => 3,
        4 => 12,
        _ => 0,
    }
}

#[start]
fn start(_argc: isize, _argv: *const *const u8) -> isize {
    // We're going to reverse it, so we need the total.
    // For now it's hardcoded, but eventually we probably want to use SPIR-V builtins.
    const TOTAL: u32 = 65535; // 0..65536

    let slot = thread_id();

    let val = *slot;
    let new_val = do_something(val);

    *slot = new_val;

    0
}

/// The playground refuses to compile it without a fake main function
/// This won't get called, though, because we're using the #[start] attribute
fn main() {}
;)
