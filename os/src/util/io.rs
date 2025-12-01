use crate::mm::PTEFlags;
use crate::util::mm::auth_check;
use alloc::vec::Vec;

impl SerializeToBytes for u8 {
}

/// structs that implement this trait can be serialized to and deserialized from byte slices
#[allow(dead_code)]
pub trait SerializeToBytes: Copy + 'static {
    /// serialize struct with specific trait to byte slice
    fn as_bytes(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts(
                self as *const Self as *const u8,
                core::mem::size_of::<Self>(),
            )
        }
    }

    /// deserialize byte slice to struct with specific trait
    fn from_bytes(bytes: &[u8]) -> Option<&Self> {
        if bytes.len() != core::mem::size_of::<Self>() {
            return None;
        }
        unsafe { Some(&*(bytes.as_ptr() as *const Self)) }
    }
}

/// write the struct to physical address range
pub fn serialize_struct<S>(s: &S, pa_range: Vec<&'static mut [u8]>) -> usize
where
    S: SerializeToBytes,
{
    let mut serialized_size = 0;
    let bytes = s.as_bytes();
    let mut bytes_written = 0;
    for slice in pa_range {
        serialized_size += slice.len();
        let len = slice.len().min(bytes.len() - bytes_written);
        slice[..len].copy_from_slice(&bytes[bytes_written..bytes_written + len]);
        bytes_written += len;
        if bytes_written >= bytes.len() {
            break;
        }
    }
    serialized_size
}

/// read `S` from physical address range
pub fn read<S>(token: usize, ptr: *const u8, len: usize) -> Result<S, &'static str>
where
    S: SerializeToBytes,
{
    let flags = PTEFlags::V | PTEFlags::R | PTEFlags::U;
    let auth = auth_check(token, ptr, len, flags);

    if !auth {
        Err("unauthorized access")
    } else {
        let pa_range = crate::util::mm::translate_va_to_pa(token, ptr, len);
        let mut data = Vec::new();
        for slice in pa_range {
            data.extend_from_slice(slice);
        }

        let x = S::from_bytes(&data).copied();
        let res = match x {
            Some(v) => Ok(v),
            None => Err("failed to deserialize"),
        };
        res
    }
}

/// write `S` to physical address range
pub fn write<S>(s: &S, token: usize, ptr: *const u8, len: usize) -> Result<usize, &'static str>
where
    S: SerializeToBytes,
{
    let flags = PTEFlags::V | PTEFlags::A | PTEFlags::R;
    let auth = auth_check(token, ptr, len, flags);
    if !auth {
        Err("unauthorized access")
    } else {
        let pa_range = crate::util::mm::translate_va_to_pa(token, ptr, len);
        let serialized_size = serialize_struct(s, pa_range);
        Ok(serialized_size)
    }
}
