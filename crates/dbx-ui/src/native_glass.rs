//! Main-thread-only AppKit host. Swift owns native views; Rust owns all DBX
//! state. Commands cross the ABI as queued JSON, never reentrant callbacks.
use gpui::Window;
use objc2::MainThreadMarker;
use objc2_app_kit::NSWorkspace;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    ffi::{CStr, c_char, c_void},
    ptr::NonNull,
    time::{Duration, Instant},
};

unsafe extern "C" {
    fn dbx_center_window(view: *mut c_void, width: f64, height: f64);
    fn dbx_native_shell_supported() -> bool;
    fn dbx_native_context_menu(
        view: *mut c_void,
        bytes: *const u8,
        count: usize,
        id: u64,
        x: f64,
        y: f64,
    ) -> bool;
    fn dbx_native_shell_create(view: *mut c_void) -> *mut c_void;
    fn dbx_native_shell_destroy(shell: *mut c_void);
    fn dbx_native_shell_sync(shell: *mut c_void, bytes: *const u8, count: usize);
    fn dbx_native_shell_drain(shell: *mut c_void) -> *mut c_char;
    fn dbx_native_shell_focus_explorer(shell: *mut c_void);
    fn dbx_native_shell_scroll(shell: *mut c_void, offset: f64, maximum: f64);
}

#[derive(Default)]
pub(crate) struct NativeGlass {
    shell: Option<NonNull<c_void>>,
    system_reduce: bool,
    dirty: bool,
    last_system_check: Option<Instant>,
    pub(crate) left_inset: f32,
    pub(crate) top_inset: f32,
}

pub(crate) fn center_window(window: &Window, width: f32, height: f32) -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    // SAFETY: GPUI owns the live view and this runs on the main thread.
    unsafe { dbx_center_window(handle.ns_view.as_ptr(), width as f64, height as f64) };
    true
}

pub(crate) fn supported() -> bool {
    // Swift availability checks cover both the SDK used to build and the OS.
    unsafe { dbx_native_shell_supported() }
}

fn system_reduce_transparency() -> bool {
    let workspace = NSWorkspace::sharedWorkspace();
    workspace.accessibilityDisplayShouldReduceTransparency()
        || workspace.accessibilityDisplayShouldIncreaseContrast()
}

impl NativeGlass {
    pub(crate) fn active(&self) -> bool {
        self.shell.is_some()
    }
    pub(crate) fn system_reduces_transparency(&self) -> bool {
        self.system_reduce
    }
    pub(crate) fn accessibility_changed(&mut self) -> bool {
        if self
            .last_system_check
            .is_some_and(|last| last.elapsed() < Duration::from_secs(1))
        {
            return false;
        }
        self.last_system_check = Some(Instant::now());
        let reduce = system_reduce_transparency();
        let changed = reduce != self.system_reduce;
        self.system_reduce = reduce;
        changed
    }
    pub(crate) fn apply(&mut self, window: &Window, enabled: bool, _dark: bool) -> bool {
        if MainThreadMarker::new().is_none() {
            return false;
        }
        self.system_reduce = system_reduce_transparency();
        self.last_system_check = Some(Instant::now());
        let enabled = enabled && supported() && !self.system_reduce;
        if enabled && self.shell.is_none() {
            let Ok(handle) = HasWindowHandle::window_handle(window) else {
                return false;
            };
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return false;
            };
            // SAFETY: GPUI owns this live NSView; Swift retains it until detach.
            self.shell = NonNull::new(unsafe { dbx_native_shell_create(handle.ns_view.as_ptr()) });
            self.top_inset = 52.;
            self.dirty = true;
        } else if !enabled && let Some(shell) = self.shell.take() {
            // SAFETY: balances passRetained in Swift; UI thread only.
            unsafe { dbx_native_shell_destroy(shell.as_ptr()) };
            self.left_inset = 0.;
            self.top_inset = 0.;
        }
        self.active()
    }
    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }
    pub(crate) fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
    pub(crate) fn scroll(&self, offset: f32, maximum: f32) {
        if let Some(shell) = self.shell {
            // SAFETY: finite GPUI geometry, main thread, no callbacks into Rust.
            unsafe { dbx_native_shell_scroll(shell.as_ptr(), offset as f64, maximum as f64) };
        }
    }
    pub(crate) fn sync(&self, model: &serde_json::Value) {
        if let Some(shell) = self.shell {
            let bytes = serde_json::to_vec(model).expect("native snapshot is JSON");
            // SAFETY: Swift copies the borrowed bytes during this call.
            unsafe { dbx_native_shell_sync(shell.as_ptr(), bytes.as_ptr(), bytes.len()) };
        }
    }
    pub(crate) fn drain(&mut self) -> Vec<serde_json::Value> {
        let Some(shell) = self.shell else {
            return Vec::new();
        };
        // SAFETY: returned strdup storage is owned by Rust and freed once.
        let raw = unsafe { dbx_native_shell_drain(shell.as_ptr()) };
        if raw.is_null() {
            return Vec::new();
        }
        let commands =
            unsafe { serde_json::from_slice(CStr::from_ptr(raw).to_bytes()) }.unwrap_or_default();
        unsafe { libc::free(raw.cast()) };
        commands
    }
    pub(crate) fn focus_explorer(&self) -> bool {
        if let Some(shell) = self.shell {
            unsafe { dbx_native_shell_focus_explorer(shell.as_ptr()) };
            true
        } else {
            false
        }
    }
}

impl Drop for NativeGlass {
    fn drop(&mut self) {
        if let Some(shell) = self.shell.take() {
            // GPUI entities are disposed on the main thread. Swift's window is
            // weak, so closing a window before its entity is safe as well.
            unsafe { dbx_native_shell_destroy(shell.as_ptr()) };
        }
    }
}

/// Present component context menus through the same AppKit menu as the explorer.
pub(crate) fn present_context_menu(
    id: u64,
    items: &serde_json::Value,
    position: gpui::Point<gpui::Pixels>,
    window: &mut Window,
    _: &mut gpui::App,
) -> bool {
    if !crate::theme::native_glass_active() || MainThreadMarker::new().is_none() {
        return false;
    }
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    let Ok(bytes) = serde_json::to_vec(items) else {
        return false;
    };
    unsafe {
        dbx_native_context_menu(
            handle.ns_view.as_ptr(),
            bytes.as_ptr(),
            bytes.len(),
            id,
            f32::from(position.x) as f64,
            f32::from(position.y) as f64,
        )
    }
}
