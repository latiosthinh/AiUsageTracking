/// Held for the life of the process; dropping it releases the mutex.
pub struct InstanceGuard {
    handle: isize,
} // 0 = no handle (non-Windows or CreateMutexW failed)

pub const MUTEX_NAME: &str = "Local\\ai-usage-widget-single-instance";

/// None if another instance already holds the mutex.
pub fn acquire() -> Option<InstanceGuard> {
    acquire_named(MUTEX_NAME)
}

pub fn acquire_named(name: &str) -> Option<InstanceGuard> {
    acquire_named_impl(name)
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn acquire_named_impl(name: &str) -> Option<InstanceGuard> {
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            lp_mutex_attributes: *const core::ffi::c_void,
            b_initial_owner: i32,
            lp_name: *const u16,
        ) -> isize;
        fn GetLastError() -> u32;
        fn CloseHandle(h_object: isize) -> i32;
    }

    const ERROR_ALREADY_EXISTS: u32 = 183;

    let name_wide = wide(name);
    // SAFETY: `name_wide` is a valid NUL-terminated wide string; attributes are null.
    let handle = unsafe { CreateMutexW(core::ptr::null(), 0, name_wide.as_ptr()) };
    // SAFETY: GetLastError has no pointer parameters; valid immediately after CreateMutexW.
    let last_error = unsafe { GetLastError() };

    if handle == 0 {
        return Some(InstanceGuard { handle: 0 });
    }

    if last_error == ERROR_ALREADY_EXISTS {
        // SAFETY: `handle` is a valid handle returned by CreateMutexW.
        unsafe {
            CloseHandle(handle);
        }
        return None;
    }

    Some(InstanceGuard { handle })
}

#[cfg(not(windows))]
fn acquire_named_impl(_name: &str) -> Option<InstanceGuard> {
    Some(InstanceGuard { handle: 0 })
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        release_handle(self.handle);
    }
}

#[cfg(windows)]
fn release_handle(handle: isize) {
    if handle == 0 {
        return;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CloseHandle(h_object: isize) -> i32;
    }

    // SAFETY: `handle` is a valid mutex handle owned by this guard.
    unsafe {
        let _ = CloseHandle(handle);
    }
}

#[cfg(not(windows))]
fn release_handle(_handle: isize) {}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn acquire_named_second_instance_gets_none() {
        let name = format!(
            "Local\\ai-usage-widget-test-{}",
            std::process::id()
        );

        let first = acquire_named(&name).expect("first acquire should succeed");
        assert!(
            acquire_named(&name).is_none(),
            "second acquire while first is held should be None"
        );
        drop(first);
        assert!(
            acquire_named(&name).is_some(),
            "third acquire after dropping first should succeed"
        );
    }
}
