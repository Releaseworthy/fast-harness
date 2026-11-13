//! Two MSVC STL helpers the prebuilt Skia library (skia-bindings, D78) calls
//! but whose MSVC 14.42 runtime library lacks: Skia's binaries were built with
//! a newer STL that vectorizes `std::search` and `find_first_of` on bytes.
//! Semantics from microsoft/STL `stl/src/vector_algorithms.cpp`. A toolset
//! that defines them (14.43 and later) takes these first: the linker pulls a
//! library member only for an unresolved symbol.

use std::ffi::c_void;

/// `std::search` on bytes: the first `count2` bytes at `first2` inside
/// `[first1, last1)`, or `last1` when absent.
///
/// # Safety
/// `[first1, last1)` and `count2` bytes at `first2` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_search_1(
    first1: *const c_void,
    last1: *const c_void,
    first2: *const c_void,
    count2: usize,
) -> *const c_void {
    if count2 == 0 {
        return first1;
    }
    let len = (last1 as usize).saturating_sub(first1 as usize);
    // SAFETY: the caller (the STL) passes two valid readable ranges of these
    // lengths; neither is written.
    let (hay, needle) = unsafe {
        (
            std::slice::from_raw_parts(first1.cast::<u8>(), len),
            std::slice::from_raw_parts(first2.cast::<u8>(), count2),
        )
    };
    match hay.windows(count2).position(|w| w == needle) {
        Some(at) => first1.cast::<u8>().wrapping_add(at).cast(),
        None => last1,
    }
}

/// `find_first_of` on bytes: the index of the first haystack byte that is in
/// the needle, or `usize::MAX` (npos).
///
/// # Safety
/// `hay_len` bytes at `hay` and `needle_len` bytes at `needle` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_find_first_of_trivial_pos_1(
    hay: *const c_void,
    hay_len: usize,
    needle: *const c_void,
    needle_len: usize,
) -> usize {
    if hay_len == 0 || needle_len == 0 {
        return usize::MAX;
    }
    // SAFETY: the caller (the STL) passes two valid readable ranges of these
    // lengths; neither is written.
    let (hay, needle) = unsafe {
        (
            std::slice::from_raw_parts(hay.cast::<u8>(), hay_len),
            std::slice::from_raw_parts(needle.cast::<u8>(), needle_len),
        )
    };
    hay.iter()
        .position(|b| needle.contains(b))
        .unwrap_or(usize::MAX)
}
