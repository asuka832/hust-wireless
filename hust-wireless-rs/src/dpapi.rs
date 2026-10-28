use crate::error::Result;

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    use crate::error::{AppError, Result};

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            data_descr: *const u16,
            optional_entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            data_in: *const DataBlob,
            data_descr: *mut *mut u16,
            optional_entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt_struct: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    pub fn crypt(data: &[u8], protect: bool) -> Result<Vec<u8>> {
        let input = DataBlob {
            cb_data: u32::try_from(data.len())
                .map_err(|_| AppError::msg("密码过长，无法交给 Windows 加密"))?,
            pb_data: data.as_ptr().cast_mut(),
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: null_mut(),
        };
        let description: Vec<u16> = "HUST wired auth\0".encode_utf16().collect();

        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &input,
                    description.as_ptr(),
                    null(),
                    null_mut(),
                    null_mut(),
                    0,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    null_mut(),
                    null(),
                    null_mut(),
                    null_mut(),
                    0,
                    &mut output,
                )
            }
        };
        if ok == 0 {
            return Err(AppError::msg(format!(
                "Windows 密码保护失败，错误码 {}",
                std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or_default()
            )));
        }

        let result = if output.cb_data == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() }
        };
        unsafe {
            LocalFree(output.pb_data.cast());
        }
        Ok(result)
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::error::{AppError, Result};

    pub fn crypt(_data: &[u8], _protect: bool) -> Result<Vec<u8>> {
        Err(AppError::msg("密码加密需要 Windows"))
    }
}

pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
    platform::crypt(data, true)
}

pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    platform::crypt(data, false)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn round_trips_password() {
        let data = b"private-password";
        let protected = protect(data).unwrap();
        assert_ne!(protected, data);
        assert_eq!(unprotect(&protected).unwrap(), data);
    }
}
