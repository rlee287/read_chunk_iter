use std::io::{Read, IoSliceMut};
use std::io::Result as IOResult;

fn chunk_slice_for_vectored_read(slice: &mut [u8], size: usize) -> Vec<IoSliceMut> {
    assert!(size > 0);

    let mut vec_slices = Vec::new();
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
