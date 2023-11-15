#![no_std]
#![feature(raw_ref_op)]
#![feature(decl_macro)]
extern crate alloc as rust_alloc;

mod alloc;
pub mod env;
pub mod hash;

#[macro_export]
macro_rules! entry {
    ($path:path) => {
        // Type check the given path
        const MOZAK_ENTRY: fn() = $path;

        mod mozak_generated_main {
            #[no_mangle]
            fn main() { super::MOZAK_ENTRY() }
        }
    };
}

#[allow(unused_macros)]
macro addr_of($place:expr) {
    &raw const $place
}

#[no_mangle]
unsafe extern "C" fn __start() {
    env::init();
    {
        extern "C" {
            fn main();
        }
        main()
    }
    env::finalize();
}

// The stack grows downwards (towards lower addresses) and the stack pointer
// shall be aligned to a 128-bit boundary upon procedure entry. The first
// argument passed on the stack is located at offset zero of the stack pointer
// on function entry; following arguments are stored at correspondingly higher
// addresses.
//
// For more details:
// https://github.com/riscv-non-isa/riscv-elf-psabi-doc/blob/master/riscv-cc.adoc
extern "C" {
    #[link_name = "_mozak_stack_top"]
    static _mozak_stack_top: u32;
    #[link_name = "_mozak_merkle_state_root"]
    static _mozak_merkle_state_root: u32;
    #[link_name = "_mozak_timestamp"]
    static _mozak_timestamp: u32;
    #[link_name = "_mozak_public_io_tape"]
    static _mozak_public_io_tape: u32;
    #[link_name = "_mozak_private_io_tape"]
    static _mozak_private_io_tape: u32;
}

// Entry point; sets up stack pointer and passes to __start.
core::arch::global_asm!(
r#"
.section .text._start;
.global _start;
_start:
    la sp, {0}
    lw sp, 0(sp)
    jal ra, __start;
"#,
    sym _mozak_stack_top
);

#[cfg(all(not(feature = "std"), target_os = "zkvm"))]
mod handlers {
    use core::panic::PanicInfo;

    #[panic_handler]
    fn panic_fault(panic_info: &PanicInfo) -> ! {
        let msg = rust_alloc::format!("{}", panic_info);
        mozak_system::system::syscall_panic(msg.as_ptr(), msg.len());
        unreachable!();
    }
}
