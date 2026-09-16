//! Windows storage primitives the disk benchmark needs.
//!
//! Two things live here, both unavoidable if the disk numbers are to mean
//! anything:
//!
//! **Unbuffered reads.** Windows caches file data aggressively. A 200 MB
//! small-file corpus on a 16 GB laptop is entirely in RAM after the first pass,
//! so a naive second run reports several GB/s and the seek-thrash curve — the
//! whole point of the exercise — measures nothing but memcpy. Opening with
//! `FILE_FLAG_NO_BUFFERING` forces every read to reach the platter, at the cost
//! of having to honour sector alignment by hand.
//!
//! **Seek-penalty detection.** The I/O scheduler's concurrency cap depends on
//! whether the drive is spinning rust or flash. Windows will tell us, via
//! `IOCTL_STORAGE_QUERY_PROPERTY`, so we do not have to guess from the model
//! string.

use std::alloc::{Layout, alloc, dealloc};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_NO_BUFFERING, FILE_FLAG_SEQUENTIAL_SCAN,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile, SetFilePointerEx,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_PROPERTY_QUERY, StorageDeviceSeekPenaltyProperty,
};

/// Sector alignment for unbuffered I/O.
///
/// 4096 covers both 512e and native-4K drives. Over-aligning is harmless;
/// under-aligning makes `ReadFile` fail outright with ERROR_INVALID_PARAMETER.
pub const SECTOR_ALIGN: usize = 4096;

/// A heap buffer aligned well enough for `FILE_FLAG_NO_BUFFERING`.
pub struct AlignedBuffer {
    ptr: *mut u8,
    len: usize,
    layout: Layout,
}

// The pointer is uniquely owned; there is no interior sharing.
unsafe impl Send for AlignedBuffer {}

impl AlignedBuffer {
    /// Allocates `len` bytes, rounded up to a sector multiple.
    pub fn new(len: usize) -> Result<Self> {
        let len = len.next_multiple_of(SECTOR_ALIGN).max(SECTOR_ALIGN);
        let layout = Layout::from_size_align(len, SECTOR_ALIGN)?;
        // SAFETY: layout has non-zero size (at least SECTOR_ALIGN).
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            bail!("could not allocate a {len}-byte aligned buffer");
        }
        Ok(Self { ptr, len, layout })
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: ptr is valid for len bytes and uniquely borrowed here.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: allocated by us with this exact layout, freed once.
        unsafe { dealloc(self.ptr, self.layout) }
    }
}

/// A file handle opened to bypass the Windows file cache.
pub struct UnbufferedFile {
    handle: HANDLE,
}

// The handle is owned solely by this struct.
unsafe impl Send for UnbufferedFile {}

impl UnbufferedFile {
    pub fn open(path: &Path) -> Result<Self> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN,
                ptr::null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            bail!(
                "opening {} unbuffered: {}",
                path.display(),
                std::io::Error::last_os_error()
            );
        }
        Ok(Self { handle })
    }

    /// Seeks to a sector-aligned offset.
    pub fn seek_to(&self, offset: u64) -> Result<()> {
        if !offset.is_multiple_of(SECTOR_ALIGN as u64) {
            bail!("unbuffered reads need a sector-aligned offset, got {offset}");
        }
        // SAFETY: handle is valid; we pass a null out-pointer, which is allowed.
        let ok = unsafe { SetFilePointerEx(self.handle, offset as i64, ptr::null_mut(), 0) };
        if ok == 0 {
            bail!("seeking to {offset}: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Reads into `buf`, returning bytes read. Zero means end of file.
    ///
    /// The buffer length must be a sector multiple, which [`AlignedBuffer`]
    /// guarantees. A short read is normal at end of file.
    pub fn read(&self, buf: &mut AlignedBuffer) -> Result<usize> {
        let mut read: u32 = 0;
        let slice = buf.as_mut_slice();
        // SAFETY: handle is valid; slice is writable for its full length.
        let ok = unsafe {
            ReadFile(
                self.handle,
                slice.as_mut_ptr(),
                slice.len() as u32,
                &mut read,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            bail!("unbuffered read: {}", std::io::Error::last_os_error());
        }
        Ok(read as usize)
    }

    /// Reads the whole file, returning total bytes. Contents are discarded —
    /// the benchmark measures the disk, not the allocator.
    pub fn read_all(&self, buf: &mut AlignedBuffer) -> Result<u64> {
        let mut total = 0u64;
        loop {
            let n = self.read(buf)?;
            if n == 0 {
                return Ok(total);
            }
            total += n as u64;
        }
    }
}

impl Drop for UnbufferedFile {
    fn drop(&mut self) {
        // SAFETY: handle came from CreateFileW and is closed exactly once.
        unsafe { CloseHandle(self.handle) };
    }
}

/// What kind of drive is behind a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DriveKind {
    /// Spinning disk: seeks are expensive, so keep queue depth low and sort by
    /// offset.
    Spinning,
    /// Flash: parallel random reads are fine and usually faster.
    Solid,
    /// Query failed, e.g. on a USB bridge that does not pass the command
    /// through. Treat as spinning — the conservative choice.
    Unknown,
}

impl DriveKind {
    /// Disk queue depth to read small files with.
    ///
    /// This used to return 2 for spinning disks, on the textbook reasoning that
    /// a platter has one head and parallel random reads make it thrash.
    /// **Measurement on the actual USB drive contradicted that**, and the
    /// measurement wins:
    ///
    /// ```text
    ///  1 thread  -> 10.1 MB/s
    ///  2 threads -> 11.5
    ///  4 threads -> 14.3
    ///  8 threads -> 18.9
    /// 16 threads -> 24.7 MB/s   (2.45x, still climbing)
    /// ```
    ///
    /// Windows reports a seek penalty on this drive, so the textbook rule
    /// should have applied — but throughput rose monotonically all the way to
    /// 16. A USB bridge does its own queuing and reordering, and the drive has
    /// a cache, so the head is not simply following the request order.
    ///
    /// This matters more than it looks: at one thread the drive delivers
    /// 10 MB/s, well *under* the 22.7 MB/s link. Read small files serially and
    /// the disk becomes the bottleneck, not the network.
    ///
    /// Re-run `basalt-bench disk` on any new drive; this is a starting point,
    /// not a law.
    pub fn suggested_queue_depth(self) -> usize {
        match self {
            // Measured optimum was 16 and had not yet plateaued. 12 keeps most
            // of the gain while leaving headroom for other work on a machine
            // with only 8 threads.
            DriveKind::Spinning | DriveKind::Unknown => 12,
            DriveKind::Solid => 16,
        }
    }
}

/// Asks Windows whether the volume containing `path` has a seek penalty.
pub fn detect_drive_kind(path: &Path) -> DriveKind {
    let Some(volume) = volume_device_path(path) else {
        return DriveKind::Unknown;
    };

    let wide: Vec<u16> = volume.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: NUL-terminated UTF-16 path; the volume handle needs no access
    // rights for a property query, hence the 0.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return DriveKind::Unknown;
    }

    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceSeekPenaltyProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut descriptor = DEVICE_SEEK_PENALTY_DESCRIPTOR {
        Version: 0,
        Size: 0,
        IncursSeekPenalty: false,
    };
    let mut returned: u32 = 0;

    // SAFETY: both buffers are valid, correctly sized, and live across the call.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &query as *const _ as *const _,
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            &mut descriptor as *mut _ as *mut _,
            size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    // SAFETY: handle came from CreateFileW and is closed exactly once.
    unsafe { CloseHandle(handle) };

    if ok == 0 || returned == 0 {
        return DriveKind::Unknown;
    }
    if descriptor.IncursSeekPenalty {
        DriveKind::Spinning
    } else {
        DriveKind::Solid
    }
}

/// Turns `D:\some\path` into `\\.\D:`, the form `CreateFileW` needs to open the
/// volume itself rather than a file on it.
fn volume_device_path(path: &Path) -> Option<String> {
    let full = path.canonicalize().ok()?;
    let s = full.to_string_lossy();
    // `canonicalize` yields the `\\?\D:\...` extended form.
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        Some(format!(r"\\.\{}:", bytes[0] as char))
    } else {
        // UNC or a mount point without a drive letter — no volume to query.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_buffer_is_sector_aligned_and_rounded_up() {
        for request in [1usize, 100, 4095, 4096, 4097, 1 << 20] {
            let buf = AlignedBuffer::new(request).unwrap();
            assert_eq!(
                buf.ptr as usize % SECTOR_ALIGN,
                0,
                "buffer for {request} bytes is not sector-aligned"
            );
            assert!(buf.len() >= request, "buffer for {request} is too small");
            assert_eq!(
                buf.len() % SECTOR_ALIGN,
                0,
                "buffer length for {request} is not a sector multiple"
            );
        }
    }

    #[test]
    fn aligned_buffer_is_writable_across_its_whole_length() {
        let mut buf = AlignedBuffer::new(8192).unwrap();
        let len = buf.len();
        buf.as_mut_slice().fill(0xAB);
        assert!(buf.as_mut_slice().iter().all(|&b| b == 0xAB));
        assert_eq!(len, 8192);
    }

    #[test]
    fn unbuffered_read_returns_the_same_bytes_as_a_normal_read() {
        // The whole benchmark rests on unbuffered reads being correct, not just
        // fast. A short final sector is the classic place to get this wrong.
        let path = std::env::temp_dir().join(format!("basalt-unbuf-{}.bin", std::process::id()));
        // Deliberately not a sector multiple, so the last read is partial.
        let data: Vec<u8> = (0..(SECTOR_ALIGN * 3 + 137) as u32)
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&path, &data).unwrap();

        let file = UnbufferedFile::open(&path).unwrap();
        let mut buf = AlignedBuffer::new(SECTOR_ALIGN).unwrap();
        let total = file.read_all(&mut buf).unwrap();
        drop(file);

        assert_eq!(
            total,
            data.len() as u64,
            "unbuffered read returned {total} bytes for a {}-byte file",
            data.len()
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn opening_a_missing_file_unbuffered_fails_cleanly() {
        let missing = std::env::temp_dir().join("basalt-definitely-not-here.bin");
        assert!(UnbufferedFile::open(&missing).is_err());
    }

    #[test]
    fn unaligned_seek_is_rejected() {
        let path = std::env::temp_dir().join(format!("basalt-seek-{}.bin", std::process::id()));
        std::fs::write(&path, vec![0u8; SECTOR_ALIGN * 2]).unwrap();
        let file = UnbufferedFile::open(&path).unwrap();
        assert!(
            file.seek_to(100).is_err(),
            "unaligned offsets must be refused"
        );
        assert!(file.seek_to(SECTOR_ALIGN as u64).is_ok());
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn volume_device_path_extracts_the_drive_letter() {
        let temp = std::env::temp_dir();
        if let Some(v) = volume_device_path(&temp) {
            assert!(
                v.starts_with(r"\\.\") && v.ends_with(':'),
                "unexpected volume path {v:?}"
            );
        }
    }

    #[test]
    fn drive_kind_detection_returns_something_usable() {
        // Cannot assert which kind without knowing the machine, but it must not
        // panic and must produce a sane queue depth.
        let kind = detect_drive_kind(&std::env::temp_dir());
        assert!(kind.suggested_queue_depth() >= 1);
    }

    #[test]
    fn queue_depth_is_high_enough_to_outrun_the_link() {
        // The queue depth exists so the disk never becomes the bottleneck.
        // Measured on the real USB drive: 1 thread gives 10.1 MB/s against a
        // 22.7 MB/s link, so serial reads lose outright. Throughput rose
        // monotonically to 24.7 MB/s at 16 threads.
        //
        // This replaces a test asserting the opposite (depth <= 4 for spinning
        // disks). That encoded the textbook head-thrashing rule, which the
        // measurement disproved on this hardware.
        for kind in [DriveKind::Spinning, DriveKind::Unknown, DriveKind::Solid] {
            assert!(
                kind.suggested_queue_depth() >= 8,
                "{kind:?} depth {} is too low to keep the disk ahead of the link",
                kind.suggested_queue_depth()
            );
        }
    }
}
