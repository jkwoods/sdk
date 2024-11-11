use serde::{ser::SerializeSeq, Deserialize, Deserializer, Serialize, Serializer};
use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    mem::size_of,
    ops::{Index, IndexMut},
    ptr,
    slice::{from_raw_parts, from_raw_parts_mut},
};

const ALIGN_SIMD: usize = 64; // enough to support AVX-512
pub type AlignedMemory64 = AlignedMemory<ALIGN_SIMD>;

pub struct AlignedMemory<const ALIGN: usize> {
    p: *mut u64,
    sz_u64: usize,
    layout: Layout,
}

impl<const ALIGN: usize> Serialize for AlignedMemory<ALIGN> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let slice: &[u64] = unsafe { from_raw_parts(self.p, self.sz_u64) };

        let mut seq = serializer.serialize_seq(Some(self.sz_u64))?;
        for &item in slice {
            seq.serialize_element(&item)?;
        }
        seq.end()
    }
}

impl<'de, const ALIGN: usize> Deserialize<'de> for AlignedMemory<ALIGN> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialize as a vector of u64s
        let vec: Vec<u64> = Vec::deserialize(deserializer)?;

        // Get the size of the vector
        let sz_u64 = vec.len();

        // Allocate memory with the required alignment
        let sz_bytes = sz_u64 * size_of::<u64>();
        let layout = Layout::from_size_align(sz_bytes, ALIGN).map_err(serde::de::Error::custom)?;

        let p = unsafe {
            let ptr = alloc_zeroed(layout) as *mut u64;
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            ptr
        };

        // Copy the deserialized values into the allocated memory
        unsafe {
            ptr::copy_nonoverlapping(vec.as_ptr(), p, sz_u64);
        }

        Ok(AlignedMemory { p, sz_u64, layout })
    }
}

impl<const ALIGN: usize> AlignedMemory<{ ALIGN }> {
    pub fn new(sz_u64: usize) -> Self {
        let sz_bytes = sz_u64 * size_of::<u64>();
        let layout = Layout::from_size_align(sz_bytes, ALIGN).unwrap();

        let ptr;
        unsafe {
            ptr = alloc_zeroed(layout);
        }

        Self {
            p: ptr as *mut u64,
            sz_u64,
            layout,
        }
    }

    // pub fn from(data: &[u8]) -> Self {
    //     let sz_u64 = (data.len() + size_of::<u64>() - 1) / size_of::<u64>();
    //     let mut out = Self::new(sz_u64);
    //     let out_slice = out.as_mut_slice();
    //     let mut i = 0;
    //     for chunk in data.chunks(size_of::<u64>()) {
    //         out_slice[i] = u64::from_ne_bytes(chunk);
    //         i += 1;
    //     }
    //     out
    // }

    pub fn as_slice(&self) -> &[u64] {
        unsafe { from_raw_parts(self.p, self.sz_u64) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u64] {
        unsafe { from_raw_parts_mut(self.p, self.sz_u64) }
    }

    pub unsafe fn as_ptr(&self) -> *const u64 {
        self.p
    }

    pub unsafe fn as_mut_ptr(&mut self) -> *mut u64 {
        self.p
    }

    pub fn len(&self) -> usize {
        self.sz_u64
    }
}

unsafe impl<const ALIGN: usize> Send for AlignedMemory<{ ALIGN }> {}
unsafe impl<const ALIGN: usize> Sync for AlignedMemory<{ ALIGN }> {}

impl<const ALIGN: usize> Drop for AlignedMemory<{ ALIGN }> {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.p as *mut u8, self.layout);
        }
    }
}

impl<const ALIGN: usize> Index<usize> for AlignedMemory<{ ALIGN }> {
    type Output = u64;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_slice()[index]
    }
}

impl<const ALIGN: usize> IndexMut<usize> for AlignedMemory<{ ALIGN }> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.as_mut_slice()[index]
    }
}

impl<const ALIGN: usize> Clone for AlignedMemory<{ ALIGN }> {
    fn clone(&self) -> Self {
        let mut out = Self::new(self.sz_u64);
        out.as_mut_slice().copy_from_slice(self.as_slice());
        out
    }
}
