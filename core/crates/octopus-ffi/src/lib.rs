//! C ABI over `octopus-core`.
//!
//! Every value crosses the boundary as UTF-8 JSON. Text arguments are
//! NUL-terminated; results come back in an [`OctopusBuf`], which the caller
//! frees with [`octopus_buf_free`]. A function that can fail returns a status
//! (`OCTOPUS_OK` or an `OCTOPUS_ERR_*` code); [`octopus_last_error`] then
//! describes the failure on the same thread as JSON `{code, type, message}`.
//! No panic crosses the boundary: each export catches it and returns
//! `OCTOPUS_ERR_PANIC`.

use std::cell::RefCell;
use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};

use octopus_core::json::{self, Value};
use octopus_core::vmap;

mod engine;

pub use engine::*;

pub const OCTOPUS_OK: i32 = 0;
/// A null, non-UTF-8 or malformed argument.
pub const OCTOPUS_ERR_ARGUMENT: i32 = 1;
/// The job, workflow, route or middleware does not exist.
pub const OCTOPUS_ERR_NOT_FOUND: i32 = 2;
/// The engine refused the call (an invalid blueprint, a store error, ...).
pub const OCTOPUS_ERR_ENGINE: i32 = 3;
/// The workflow failed. The result buffer still holds the job's state when
/// there is one.
pub const OCTOPUS_ERR_WORKFLOW: i32 = 4;
/// The core panicked. The engine may be unusable afterwards.
pub const OCTOPUS_ERR_PANIC: i32 = 5;

/// Bytes owned by this library.
#[repr(C)]
pub struct OctopusBuf {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl OctopusBuf {
    fn from_vec(v: Vec<u8>) -> Self {
        let mut v = std::mem::ManuallyDrop::new(v);
        OctopusBuf {
            ptr: v.as_mut_ptr(),
            len: v.len(),
            cap: v.capacity(),
        }
    }

    fn empty() -> Self {
        Self::from_vec(Vec::new())
    }
}

/// A failure, as `octopus_last_error` reports it.
pub(crate) struct Error {
    pub status: i32,
    pub kind: String,
    pub message: String,
}

impl Error {
    pub fn new(status: i32, kind: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            kind: kind.to_string(),
            message: message.into(),
        }
    }

    pub fn argument(message: impl Into<String>) -> Self {
        Self::new(OCTOPUS_ERR_ARGUMENT, "InvalidArgument", message)
    }

    pub fn engine(message: impl Into<String>) -> Self {
        Self::new(OCTOPUS_ERR_ENGINE, "EngineError", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(OCTOPUS_ERR_NOT_FOUND, "NotFound", message)
    }
}

thread_local! {
    static LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_last_error(err: &Error) {
    let text = Value::Object(vmap! {
        "code" => err.status as i64,
        "type" => err.kind.clone(),
        "message" => err.message.clone(),
    })
    .to_json_string();
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(text));
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_string())
}

/// Runs an export's body: clears the last error, records a new one, and turns
/// a panic into `OCTOPUS_ERR_PANIC`.
pub(crate) fn guard(f: impl FnOnce() -> Result<i32, Error>) -> i32 {
    LAST_ERROR.with(|e| *e.borrow_mut() = None);
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => {
            set_last_error(&err);
            err.status
        }
        Err(payload) => {
            set_last_error(&Error::new(
                OCTOPUS_ERR_PANIC,
                "Panic",
                panic_message(payload),
            ));
            OCTOPUS_ERR_PANIC
        }
    }
}

/// Borrows a NUL-terminated UTF-8 argument.
pub(crate) unsafe fn text<'a>(p: *const c_char, what: &str) -> Result<&'a str, Error> {
    if p.is_null() {
        return Err(Error::argument(format!("{} is null", what)));
    }
    CStr::from_ptr(p)
        .to_str()
        .map_err(|_| Error::argument(format!("{} is not UTF-8", what)))
}

/// Like [`text`], with null meaning "not given".
pub(crate) unsafe fn opt_text<'a>(p: *const c_char, what: &str) -> Result<Option<&'a str>, Error> {
    if p.is_null() {
        Ok(None)
    } else {
        text(p, what).map(Some)
    }
}

/// Parses a JSON argument; null means `null`.
pub(crate) unsafe fn json_arg(p: *const c_char, what: &str) -> Result<Value, Error> {
    match opt_text(p, what)? {
        None => Ok(Value::Null),
        Some(t) => json::parse(t).map_err(|e| Error::argument(format!("{}: {}", what, e))),
    }
}

/// Stores a result for the caller.
pub(crate) unsafe fn write_out(out: *mut OctopusBuf, bytes: Vec<u8>) -> Result<(), Error> {
    if out.is_null() {
        return Err(Error::argument("the result pointer is null"));
    }
    out.write(OctopusBuf::from_vec(bytes));
    Ok(())
}

pub(crate) unsafe fn write_json(out: *mut OctopusBuf, v: &Value) -> Result<(), Error> {
    write_out(out, v.to_json_string().into_bytes())
}

/// The core's crate version, as UTF-8 text.
#[no_mangle]
pub extern "C" fn octopus_core_version() -> OctopusBuf {
    OctopusBuf::from_vec(octopus_core::VERSION.as_bytes().to_vec())
}

/// The contract version SDKs check before using any other function.
#[no_mangle]
pub extern "C" fn octopus_contract_version() -> u32 {
    octopus_core::CONTRACT_VERSION
}

/// The last failure on this thread as JSON `{code, type, message}`, or an
/// empty buffer when the last call succeeded.
#[no_mangle]
pub extern "C" fn octopus_last_error() -> OctopusBuf {
    LAST_ERROR.with(|e| match e.borrow().as_ref() {
        Some(text) => OctopusBuf::from_vec(text.clone().into_bytes()),
        None => OctopusBuf::empty(),
    })
}

/// Frees a buffer returned by this library.
///
/// # Safety
/// `buf` must have come from this library and must not be used or freed again.
#[no_mangle]
pub unsafe extern "C" fn octopus_buf_free(buf: OctopusBuf) {
    if !buf.ptr.is_null() {
        drop(Vec::from_raw_parts(buf.ptr, buf.len, buf.cap));
    }
}

#[cfg(test)]
mod tests;
