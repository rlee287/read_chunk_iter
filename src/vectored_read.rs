use std::io::{Read, IoSliceMut};
use std::io::Result as IOResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VectoredReadSelect {
    Yes,
    #[cfg(feature = "autodetect_vectored")]
    Auto,
    No,
}
impl Default for VectoredReadSelect {
    #[inline]
    fn default() -> Self {
        Self::No
    }
}

// Mark the following function as always-inline because it will almost always be
// a constant for a given Read type, and we want to avoid the overhead of
// calling into this function in debug mode.

/// Returns whether to use vectored read. We do not cache this for 2 reasons:
/// 1. This may change, e.g. for chained reading where only some of them
///    have vectored reads.
/// 2. If it doesn't change then this will all be const folded away.
#[inline(always)]
#[cfg_attr(not(feature = "autodetect_vectored"), allow(unused_variables))]
pub(crate) fn resolve_read_vectored<R: Read>(reader: &R, select: VectoredReadSelect) -> bool {
    match select {
        VectoredReadSelect::Yes => true,
        #[cfg(feature = "autodetect_vectored")]
        VectoredReadSelect::Auto => reader.is_read_vectored(),
        _ => false, // includes No
    }
}

fn chunk_slice_for_vectored_read(slice: &mut [u8], size: usize) -> Vec<IoSliceMut> {
    assert!(size > 0);

    let mut vec_slices = Vec::with_capacity(slice.len().div_ceil(size));
    let mut cdr = slice;

    while cdr.len() > size {
        let (car, cdr_new) = cdr.split_at_mut(size);
        vec_slices.push(IoSliceMut::new(car));
        cdr = cdr_new;
    }
    if cdr.len() > 0 {
        vec_slices.push(IoSliceMut::new(cdr));
    }
    vec_slices
}
pub(crate) fn read_vectored_into_buf<R: Read>(reader: &mut R, slice: &mut [u8], size: usize) -> IOResult<usize> {
    let mut vec_slices = chunk_slice_for_vectored_read(slice, size);
    reader.read_vectored(&mut vec_slices)
}
