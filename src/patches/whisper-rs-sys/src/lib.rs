#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// Concat: with GGML_BACKEND_DL=ON the ggml_cpu_has_* probes live in the
// ggml-cpu backend module, which is loaded at runtime and is not linked
// into this crate. whisper-rs still calls them for SystemInfo, so provide
// them here with runtime CPU detection instead of the obsolete compile-time
// constants. The public Rust API (SystemInfo fields) is preserved.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[no_mangle]
pub extern "C" fn ggml_cpu_has_avx() -> ::std::os::raw::c_int {
    let r = unsafe { core::arch::x86_64::__cpuid_count(1, 0) };
    if r.ecx & (1 << 28) != 0 { 1 } else { 0 }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[no_mangle]
pub extern "C" fn ggml_cpu_has_avx2() -> ::std::os::raw::c_int {
    let r = unsafe { core::arch::x86_64::__cpuid_count(7, 0) };
    if r.ebx & (1 << 5) != 0 { 1 } else { 0 }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[no_mangle]
pub extern "C" fn ggml_cpu_has_fma() -> ::std::os::raw::c_int {
    let r = unsafe { core::arch::x86_64::__cpuid_count(1, 0) };
    if r.ecx & (1 << 12) != 0 { 1 } else { 0 }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[no_mangle]
pub extern "C" fn ggml_cpu_has_f16c() -> ::std::os::raw::c_int {
    let r = unsafe { core::arch::x86_64::__cpuid_count(1, 0) };
    if r.ecx & (1 << 29) != 0 { 1 } else { 0 }
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
#[no_mangle]
pub extern "C" fn ggml_cpu_has_avx() -> ::std::os::raw::c_int {
    0
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
#[no_mangle]
pub extern "C" fn ggml_cpu_has_avx2() -> ::std::os::raw::c_int {
    0
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
#[no_mangle]
pub extern "C" fn ggml_cpu_has_fma() -> ::std::os::raw::c_int {
    0
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
#[no_mangle]
pub extern "C" fn ggml_cpu_has_f16c() -> ::std::os::raw::c_int {
    0
}
