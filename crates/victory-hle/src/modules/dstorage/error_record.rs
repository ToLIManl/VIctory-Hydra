//! Approach after XCL 2e17d3d: the first failed request carries the source
//! offset, size and cancellation tag. The queue keeps this record until the
//! caller retrieves it, independently of status-array entries.

use super::{HRESULT, MEMORY_SOURCE, QueueRef, Request, WALLS_FILES};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::{LazyLock, Mutex};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent, SetEvent};

pub(super) fn enabled() -> bool {
    static ON: LazyLock<bool> = LazyLock::new(|| {
        std::env::var_os("VICTORY_DSTORAGE_ERROR_RECORD").is_some_and(|v| v == "1")
    });
    *ON
}

#[repr(C)]
pub(super) struct Record {
    count: u32,
    reserved: u32,
    hr: HRESULT,
    command: u32,
    options: u64,
    destination: u64,
    destination_size: u32,
    file: [u16; 260],
    reserved_file: u32,
    offset: u64,
    memory_source: u64,
    source_size: u32,
    reserved_source: u32,
    tag: u64,
    name: [u8; 64],
}

const _: () =
    assert!(size_of::<Record>() == 0x290 && std::mem::offset_of!(Record, offset) == 0x230);

impl Record {
    fn empty() -> Self {
        // SAFETY: all fields are integers or arrays of integers.
        Self {
            command: u32::MAX,
            ..unsafe { std::mem::zeroed() }
        }
    }

    fn note(&mut self, r: &Request, hr: HRESULT, file_name: Option<&str>) {
        self.count = self.count.saturating_add(1);
        if self.count != 1 {
            return;
        }
        self.command = 0;
        self.hr = hr;
        self.options = r.options;
        self.destination = r.destination as u64;
        self.destination_size = r.destination_size;
        self.source_size = r.source_size;
        self.tag = r.tag;
        if r.options & MEMORY_SOURCE != 0 {
            self.memory_source = r.source as u64;
        } else {
            self.offset = r.offset;
            let files = WALLS_FILES.lock().unwrap_or_else(|e| e.into_inner());
            let path = file_name.or_else(|| {
                files
                    .iter()
                    .rev()
                    .find(|(address, _)| *address == r.source as usize)
                    .map(|(_, path)| path.as_str())
            });
            if let Some(path) = path {
                for (out, unit) in self.file[..259].iter_mut().zip(path.encode_utf16()) {
                    *out = unit;
                }
            }
        }
    }
}

pub(super) struct Errors {
    record: Mutex<Record>,
    event: OwnedHandle,
}

impl Errors {
    pub(super) fn new() -> Option<Self> {
        // SAFETY: unnamed manual-reset event, initially clear.
        let h = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if h.is_null() {
            return None;
        }
        Some(Self {
            record: Mutex::new(Record::empty()),
            event: unsafe { OwnedHandle::from_raw_handle(h) },
        })
    }

    pub(super) fn note(&self, r: &Request, hr: HRESULT, file_name: Option<&str>) {
        let mut record = self.record.lock().unwrap_or_else(|e| e.into_inner());
        record.note(r, hr, file_name);
        unsafe { SetEvent(self.event.as_raw_handle()) };
    }

    fn take(&self) -> Record {
        let mut record = self.record.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { ResetEvent(self.event.as_raw_handle()) };
        std::mem::replace(&mut *record, Record::empty())
    }
}

pub(super) extern "system" fn event(queue: QueueRef) -> HANDLE {
    use std::os::windows::io::IntoRawHandle;
    unsafe { &*queue }
        .state
        .errors
        .as_ref()
        .and_then(|e| e.event.try_clone().ok())
        .map_or(std::ptr::null_mut(), IntoRawHandle::into_raw_handle)
}

pub(super) extern "system" fn retrieve(queue: QueueRef, out: *mut Record) {
    if !out.is_null()
        && let Some(errors) = &unsafe { &*queue }.state.errors
    {
        // SAFETY: caller supplies space for the console record.
        unsafe { out.write_unaligned(errors.take()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    #[test]
    fn retrieval_reports_first_request_and_clears_the_shared_event() {
        let errors = Errors::new().unwrap();
        let observer = errors.event.try_clone().unwrap();
        let mut r = Request {
            options: 1,
            destination: std::ptr::null_mut(),
            destination_size: 256,
            page_offset: 0,
            source: std::ptr::null(),
            offset: 0x1_2345_6789,
            source_size: 100,
            intermediate_size: 0,
            tag: 37,
            name: std::ptr::null(),
        };
        errors.note(&r, -17, Some("source.bin"));
        r.options = MEMORY_SOURCE;
        r.tag = 99;
        errors.note(&r, -23, None);
        assert_eq!(
            unsafe { WaitForSingleObject(observer.as_raw_handle(), 0) },
            0
        );
        let rec = errors.take();
        assert_eq!(String::from_utf16(&rec.file[..10]).unwrap(), "source.bin");
        assert_eq!(
            (rec.count, rec.hr, rec.tag, rec.offset, rec.source_size),
            (2, -17, 37, r.offset, 100)
        );
        assert_eq!(
            unsafe { WaitForSingleObject(observer.as_raw_handle(), 0) },
            258
        );
        assert_eq!(errors.take().command, u32::MAX);
        r.source = (&r as *const Request).cast();
        errors.note(&r, -23, None);
        let rec = errors.take();
        assert_eq!(
            (rec.memory_source, rec.offset, rec.tag),
            (r.source as u64, 0, 99)
        );
    }
}
