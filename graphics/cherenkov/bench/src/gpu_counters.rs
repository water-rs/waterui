//! Mali hardware counters per measured frame (`measure --gpu-counters`).
//!
//! A frame whose GPU time is a fraction of a millisecond is latency-bound
//! on the Pixel's Mali: added work can be absorbed or change the GPU's
//! power state, so its GPU time is not a cost measure at 2% (#2775). The
//! shader cores' active cycles count the cycles a core spent executing
//! work, whatever the queue and power-up latency around it, so a frame's
//! cost is the active cycles its work added.
//!
//! The counters come from the kbase driver's `kinstr_prfcnt` interface on
//! `/dev/mali0`, in manual mode: every sample holds what accumulated since
//! the previous one. The frame loop samples with the GPU idle before the
//! first measured frame and after each measured frame, so each delta is
//! exactly one frame's GPU work. The counters are global to the GPU:
//! another process's GPU work in the window counts too, which the screen-off
//! measurement protocol keeps out.
//!
//! Counter indices are per GPU product; this reads the Mali-G715 layout
//! (Valhall, CSF front end) and fails on any other GPU rather than read
//! another layout's counters under these names.

use crate::BenchError;
use crate::report::{GpuCounterSample, GpuCountersInfo};

/// An open counter session on the device's Mali GPU.
#[derive(Debug)]
pub struct Session {
    #[cfg(target_os = "android")]
    inner: kbase::Session,
    #[cfg(not(target_os = "android"))]
    never: std::convert::Infallible,
}

impl Session {
    /// Opens a manual-sampling session on `/dev/mali0` and starts it.
    ///
    /// # Errors
    /// The device has no kbase Mali GPU, the GPU is not a Mali-G715, or a
    /// driver call fails.
    #[cfg(target_os = "android")]
    pub fn open() -> Result<Self, BenchError> {
        kbase::Session::open().map(|inner| Self { inner })
    }

    /// GPU counters come from the Mali kbase driver, which only Android
    /// devices here have.
    ///
    /// # Errors
    /// Always.
    #[cfg(not(target_os = "android"))]
    pub fn open() -> Result<Self, BenchError> {
        Err(BenchError::Engine(
            "--gpu-counters reads the Mali kbase driver; this platform has none".into(),
        ))
    }

    /// The GPU, shader-core count and L2 slice count the session reads.
    #[must_use]
    #[cfg_attr(
        not(target_os = "android"),
        expect(
            clippy::missing_const_for_fn,
            reason = "const only where the session is uninhabited; Android reads the driver"
        )
    )]
    pub fn info(&self) -> GpuCountersInfo {
        #[cfg(target_os = "android")]
        {
            self.inner.info()
        }
        #[cfg(not(target_os = "android"))]
        match self.never {}
    }

    /// Takes a sample: the counters accumulated since the previous one.
    /// Call it with the GPU idle so the delta covers whole submissions.
    ///
    /// # Errors
    /// The driver call fails, or the sample is flagged as overflowed or in
    /// error.
    #[cfg_attr(
        not(target_os = "android"),
        expect(
            clippy::missing_const_for_fn,
            reason = "const only where the session is uninhabited; Android reads the driver"
        )
    )]
    pub fn sample(&mut self) -> Result<GpuCounterSample, BenchError> {
        #[cfg(target_os = "android")]
        {
            self.inner.sample()
        }
        #[cfg(not(target_os = "android"))]
        match self.never {}
    }
}

#[cfg(target_os = "android")]
mod kbase {
    //! The kbase `ioctl` interface: a system-monitor context on
    //! `/dev/mali0`, then a `kinstr_prfcnt` reader whose samples live in a
    //! kernel ring buffer mapped into this process. Layouts follow the
    //! kbase UAPI (`mali_kbase_ioctl.h`, `mali_kbase_hwcnt_reader.h`).

    use std::ffi::c_void;
    use std::fs::{File, OpenOptions};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::OpenOptionsExt;

    use crate::BenchError;
    use crate::report::{GpuCounterSample, GpuCountersInfo};

    const DEVICE: &str = "/dev/mali0";

    /// `_IOC(dir, type, nr, size)` as the Linux generic encoding defines
    /// it, as the request type bionic's `ioctl` takes.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "UAPI argument sizes are a few dozen bytes, far inside the 14-bit size field"
    )]
    const fn ioc(dir: u32, kind: u32, nr: u32, size: usize) -> libc::Ioctl {
        ((dir << 30) | ((size as u32) << 16) | (kind << 8) | nr).cast_signed()
    }
    const WRITE: u32 = 1;
    const READ: u32 = 2;
    const KBASE: u32 = 0x80;
    const READER: u32 = 0xbf;

    const VERSION_CHECK_CSF: libc::Ioctl =
        ioc(READ | WRITE, KBASE, 0x34, size_of::<VersionCheck>());
    const SET_FLAGS: libc::Ioctl = ioc(WRITE, KBASE, 0x1, size_of::<u32>());
    const GET_GPUPROPS: libc::Ioctl = ioc(WRITE, KBASE, 0x3, size_of::<GetGpuProps>());
    const ENUM_INFO: libc::Ioctl = ioc(READ | WRITE, KBASE, 0x38, size_of::<EnumInfo>());
    const SETUP: libc::Ioctl = ioc(READ | WRITE, KBASE, 0x39, size_of::<Setup>());
    const ISSUE_COMMAND: libc::Ioctl = ioc(WRITE, READER, 0x0, size_of::<ControlCmd>());
    const GET_SAMPLE: libc::Ioctl = ioc(READ, READER, 0x1, size_of::<SampleAccess>());
    const PUT_SAMPLE: libc::Ioctl = ioc(WRITE, READER, 0x10, size_of::<SampleAccess>());

    /// `BASEP_CONTEXT_FLAG`: a context that never submits work, for
    /// system monitors.
    const CONTEXT_SYSTEM_MONITOR_SUBMIT_DISABLED: u32 = 1 << 1;
    /// `KBASE_GPUPROP_RAW_GPU_ID`.
    const PROP_RAW_GPU_ID: u32 = 55;

    // Flex-list item types: `(list << 12) | subtype`.
    const ENUM_BLOCK: u16 = 0x0000;
    const REQUEST_MODE: u16 = 0x1000;
    const REQUEST_ENABLE: u16 = 0x1001;
    const METADATA_NONE: u16 = 0x0000;
    const METADATA_SAMPLE: u16 = 0x2000;
    const METADATA_BLOCK: u16 = 0x2002;

    // `prfcnt_block_type`.
    const BLOCK_FE: u8 = 0;
    const BLOCK_TILER: u8 = 1;
    const BLOCK_MEMORY: u8 = 2;
    const BLOCK_SHADER_CORE: u8 = 3;
    /// `PRFCNT_SET_PRIMARY`: the counter set the indices below name.
    const SET_PRIMARY: u8 = 0;

    // `prfcnt_control_cmd_code`.
    const CMD_START: u16 = 1;
    const CMD_STOP: u16 = 2;
    const CMD_SAMPLE_SYNC: u16 = 3;

    // `SAMPLE_FLAG_OVERFLOW`, `SAMPLE_FLAG_ERROR`.
    const SAMPLE_FLAGS_INVALID: u32 = (1 << 0) | (1 << 30);

    /// `(arch_major, product_major)` of the Mali-G715 in its GPU ID.
    const G715: (u64, u64) = (11, 2);

    // Mali-G715 primary-set counter indices within each block.
    // Arm libGPUCounters 378c7b59, valhall.cpp g615_data (also G715).
    const L2_EXT_READ_BEATS: usize = 32;
    const L2_EXT_WRITE_BEATS: usize = 47;
    const FE_GPU_ACTIVE: usize = 4;
    const TILER_ACTIVE: usize = 4;
    const SC_FRAGMENT_ACTIVE: usize = 4;
    const SC_FRAGMENT_WARPS: usize = 9;
    const SC_NON_FRAGMENT_ACTIVE: usize = 22;
    const SC_CORE_ACTIVE: usize = 26;

    #[repr(C)]
    struct VersionCheck {
        major: u16,
        minor: u16,
    }

    #[repr(C)]
    struct GetGpuProps {
        buffer: u64,
        size: u32,
        flags: u32,
    }

    #[repr(C)]
    struct EnumInfo {
        item_size: u32,
        item_count: u32,
        list: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ItemHeader {
        kind: u16,
        version: u16,
        pad: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EnumBlock {
        header: ItemHeader,
        block: u8,
        set: u8,
        pad: [u8; 2],
        instances: u16,
        values: u16,
        mask: [u64; 2],
    }

    /// A request item: the union is as large as `enable`, the largest
    /// member, which `mode` shares the layout of up to its period.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RequestItem {
        header: ItemHeader,
        /// `mode` (0 manual) for a mode item, the block type for enable.
        first: u8,
        /// The counter set for an enable item.
        set: u8,
        pad: [u8; 6],
        /// The period for a periodic mode item; the enable mask otherwise.
        words: [u64; 2],
    }

    #[repr(C)]
    union Setup {
        input: SetupIn,
        output: SetupOut,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SetupIn {
        count: u32,
        size: u32,
        requests: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SetupOut {
        metadata_item_size: u32,
        mmap_size: u32,
    }

    #[repr(C)]
    struct ControlCmd {
        cmd: u16,
        pad: [u16; 3],
        user_data: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SampleAccess {
        sequence: u64,
        metadata_offset: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SampleMetadata {
        header: ItemHeader,
        timestamp_start: u64,
        timestamp_stop: u64,
        seq: u64,
        user_data: u64,
        flags: u32,
        pad: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct BlockMetadata {
        header: ItemHeader,
        block: u8,
        index: u8,
        set: u8,
        pad: u8,
        state: u32,
        values_offset: u32,
        pad2: u32,
    }

    fn os_error(what: &str) -> BenchError {
        BenchError::Gpu(format!(
            "gpu counters: {what}: {}",
            std::io::Error::last_os_error()
        ))
    }

    /// Issues `request` on `fd` with `arg`.
    ///
    /// # Safety
    /// `request` must be a kbase command whose argument is a `T`.
    unsafe fn ioctl<T>(
        fd: i32,
        request: libc::Ioctl,
        arg: *mut T,
        what: &str,
    ) -> Result<i32, BenchError> {
        // SAFETY: the caller pairs `request` with its argument type.
        let result = unsafe { libc::ioctl(fd, request, arg) };
        if result < 0 {
            return Err(os_error(what));
        }
        Ok(result)
    }

    /// The kernel's mapping of the reader's sample buffer.
    #[derive(Debug)]
    struct Mapping {
        base: *const u8,
        len: usize,
    }

    impl Drop for Mapping {
        fn drop(&mut self) {
            // SAFETY: `base`/`len` are the mapping `mmap` returned, unmapped once.
            unsafe { libc::munmap(self.base.cast_mut().cast::<c_void>(), self.len) };
        }
    }

    /// What the reader's samples hold.
    #[derive(Debug)]
    struct Layout {
        shader_cores: u16,
        l2_slices: u16,
        metadata_item_size: usize,
    }

    #[derive(Debug)]
    pub(super) struct Session {
        // Field order is drop order: the mapping before the reader fd, the
        // reader before the context it belongs to.
        mapping: Mapping,
        reader: OwnedFd,
        _device: File,
        layout: Layout,
    }

    impl Session {
        pub(super) fn open() -> Result<Self, BenchError> {
            let device = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_CLOEXEC)
                .open(DEVICE)
                .map_err(|e| BenchError::Gpu(format!("gpu counters: {DEVICE}: {e}")))?;
            let fd = device.as_raw_fd();
            let mut version = VersionCheck { major: 0, minor: 0 };
            // SAFETY: VERSION_CHECK_CSF takes a `VersionCheck`.
            unsafe { ioctl(fd, VERSION_CHECK_CSF, &raw mut version, "CSF version check")? };
            let mut flags = CONTEXT_SYSTEM_MONITOR_SUBMIT_DISABLED;
            // SAFETY: SET_FLAGS takes a `u32`.
            unsafe { ioctl(fd, SET_FLAGS, &raw mut flags, "set context flags")? };
            check_product(fd)?;
            let (shader_cores, l2_slices) = enumerate(fd)?;
            let mut requests = [
                RequestItem {
                    header: ItemHeader {
                        kind: REQUEST_MODE,
                        version: 0,
                        pad: 0,
                    },
                    first: 0,
                    set: 0,
                    pad: [0; 6],
                    words: [0; 2],
                },
                enable(BLOCK_FE),
                enable(BLOCK_TILER),
                enable(BLOCK_MEMORY),
                enable(BLOCK_SHADER_CORE),
                RequestItem {
                    header: ItemHeader {
                        kind: 0,
                        version: 0,
                        pad: 0,
                    },
                    first: 0,
                    set: 0,
                    pad: [0; 6],
                    words: [0; 2],
                },
            ];
            let mut setup = Setup {
                input: SetupIn {
                    count: u32::try_from(requests.len()).expect("six request items"),
                    size: u32::try_from(size_of::<RequestItem>()).expect("a 32-byte item"),
                    requests: requests.as_mut_ptr() as u64,
                },
            };
            // SAFETY: SETUP takes a `Setup` whose request list outlives the call.
            let reader = unsafe { ioctl(fd, SETUP, &raw mut setup, "kinstr_prfcnt setup")? };
            // SAFETY: a successful SETUP returns a new file descriptor this
            // process owns.
            let reader = unsafe { OwnedFd::from_raw_fd(reader) };
            // SAFETY: SETUP wrote its output member.
            let out = unsafe { setup.output };
            let len = usize::try_from(out.mmap_size).expect("u32 fits usize");
            // SAFETY: maps the reader's sample buffer read-only, as the
            // driver offers it; the result is checked before use.
            let base = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len,
                    libc::PROT_READ,
                    libc::MAP_PRIVATE,
                    reader.as_raw_fd(),
                    0,
                )
            };
            if base == libc::MAP_FAILED {
                return Err(os_error("map the sample buffer"));
            }
            let session = Self {
                mapping: Mapping {
                    base: base.cast::<u8>().cast_const(),
                    len,
                },
                reader,
                _device: device,
                layout: Layout {
                    shader_cores,
                    l2_slices,
                    metadata_item_size: usize::try_from(out.metadata_item_size)
                        .expect("u32 fits usize"),
                },
            };
            session.command(CMD_START)?;
            Ok(session)
        }

        pub(super) fn info(&self) -> GpuCountersInfo {
            GpuCountersInfo {
                gpu: "Mali-G715",
                shader_cores: u32::from(self.layout.shader_cores),
                l2_slices: u32::from(self.layout.l2_slices),
            }
        }

        fn command(&self, cmd: u16) -> Result<(), BenchError> {
            let mut command = ControlCmd {
                cmd,
                pad: [0; 3],
                user_data: 0,
            };
            // SAFETY: ISSUE_COMMAND takes a `ControlCmd`.
            unsafe {
                ioctl(
                    self.reader.as_raw_fd(),
                    ISSUE_COMMAND,
                    &raw mut command,
                    "reader command",
                )?;
            }
            Ok(())
        }

        pub(super) fn sample(&self) -> Result<GpuCounterSample, BenchError> {
            self.command(CMD_SAMPLE_SYNC)?;
            let fd = self.reader.as_raw_fd();
            let mut ready = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: polls one valid descriptor.
            let polled = unsafe { libc::poll(&raw mut ready, 1, 1000) };
            if polled < 0 {
                return Err(os_error("wait for the sample"));
            }
            if polled == 0 {
                return Err(BenchError::Gpu(
                    "gpu counters: the synchronous sample was not ready within 1 s".into(),
                ));
            }
            let mut access = SampleAccess {
                sequence: 0,
                metadata_offset: 0,
            };
            // SAFETY: GET_SAMPLE takes a `SampleAccess`.
            unsafe { ioctl(fd, GET_SAMPLE, &raw mut access, "get sample")? };
            let parsed = self.parse(access.metadata_offset);
            // SAFETY: PUT_SAMPLE takes the `SampleAccess` GET_SAMPLE filled.
            unsafe { ioctl(fd, PUT_SAMPLE, &raw mut access, "put sample")? };
            parsed
        }

        /// Reads `T` at `offset` in the sample buffer.
        fn read<T: Copy>(&self, offset: usize) -> Result<T, BenchError> {
            if offset + size_of::<T>() > self.mapping.len {
                return Err(BenchError::Gpu(format!(
                    "gpu counters: the driver pointed past the sample buffer ({offset})"
                )));
            }
            // SAFETY: the range lies inside the mapping, checked above, and
            // is copied out without an alignment assumption.
            Ok(unsafe { self.mapping.base.add(offset).cast::<T>().read_unaligned() })
        }

        fn parse(&self, metadata_offset: u64) -> Result<GpuCounterSample, BenchError> {
            let mut offset = usize::try_from(metadata_offset).expect("an offset in the mapping");
            let mut sample = GpuCounterSample::default();
            let mut cores = 0u16;
            let mut l2_slices = 0u16;
            let mut saw_sample = false;
            loop {
                let header: ItemHeader = self.read(offset)?;
                match header.kind {
                    METADATA_NONE => break,
                    METADATA_SAMPLE => {
                        let meta: SampleMetadata = self.read(offset)?;
                        if meta.flags & SAMPLE_FLAGS_INVALID != 0 {
                            return Err(BenchError::Gpu(format!(
                                "gpu counters: sample {} flagged {:#x} (overflow or error)",
                                meta.seq, meta.flags
                            )));
                        }
                        saw_sample = true;
                    }
                    METADATA_BLOCK => {
                        let block: BlockMetadata = self.read(offset)?;
                        if block.set != SET_PRIMARY {
                            return Err(BenchError::Gpu(format!(
                                "gpu counters: block {} reports set {}, not the primary set",
                                block.block, block.set
                            )));
                        }
                        let values = usize::try_from(block.values_offset).expect("u32 fits usize");
                        let value = |index: usize| self.read::<u64>(values + 8 * index);
                        match block.block {
                            BLOCK_FE => sample.gpu_active_cycles += value(FE_GPU_ACTIVE)?,
                            BLOCK_TILER => sample.tiler_active_cycles += value(TILER_ACTIVE)?,
                            BLOCK_MEMORY => {
                                l2_slices += 1;
                                sample.external_read_beats += value(L2_EXT_READ_BEATS)?;
                                sample.external_write_beats += value(L2_EXT_WRITE_BEATS)?;
                            }
                            BLOCK_SHADER_CORE => {
                                cores += 1;
                                sample.shader_core_active_cycles += value(SC_CORE_ACTIVE)?;
                                sample.fragment_active_cycles += value(SC_FRAGMENT_ACTIVE)?;
                                sample.non_fragment_active_cycles += value(SC_NON_FRAGMENT_ACTIVE)?;
                                sample.fragment_warps += value(SC_FRAGMENT_WARPS)?;
                            }
                            _ => {}
                        }
                    }
                    // Clock metadata: the cycle counts of the clock domains.
                    _ => {}
                }
                offset += self.layout.metadata_item_size;
            }
            if !saw_sample
                || cores != self.layout.shader_cores
                || l2_slices != self.layout.l2_slices
            {
                return Err(BenchError::Gpu(format!(
                    "gpu counters: a sample carried {cores} of {} shader-core blocks and \
                     {l2_slices} of {} L2 blocks{}",
                    self.layout.shader_cores,
                    self.layout.l2_slices,
                    if saw_sample {
                        ""
                    } else {
                        " and no sample metadata"
                    }
                )));
            }
            Ok(sample)
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            if let Err(e) = self.command(CMD_STOP) {
                tracing::warn!("{e}");
            }
        }
    }

    const fn enable(block: u8) -> RequestItem {
        RequestItem {
            header: ItemHeader {
                kind: REQUEST_ENABLE,
                version: 0,
                pad: 0,
            },
            first: block,
            set: SET_PRIMARY,
            pad: [0; 6],
            words: [u64::MAX; 2],
        }
    }

    /// Fails unless the GPU is a Mali-G715, the layout the indices name.
    fn check_product(fd: i32) -> Result<(), BenchError> {
        let mut props = GetGpuProps {
            buffer: 0,
            size: 0,
            flags: 0,
        };
        // SAFETY: GET_GPUPROPS with size 0 only returns the buffer size.
        let size = unsafe { ioctl(fd, GET_GPUPROPS, &raw mut props, "gpu properties size")? };
        let mut buffer = vec![0u8; usize::try_from(size).expect("a positive size")];
        props.buffer = buffer.as_mut_ptr() as u64;
        props.size = u32::try_from(buffer.len()).expect("a u32 size");
        // SAFETY: the buffer is `size` bytes and outlives the call.
        unsafe { ioctl(fd, GET_GPUPROPS, &raw mut props, "gpu properties")? };
        let mut rest = buffer.as_slice();
        while rest.len() >= 4 {
            let key = u32::from_le_bytes(rest[..4].try_into().expect("four bytes"));
            let width = 1usize << (key & 3);
            let Some(bytes) = rest.get(4..4 + width) else {
                break;
            };
            let mut value = [0u8; 8];
            value[..width].copy_from_slice(bytes);
            let value = u64::from_le_bytes(value);
            if key >> 2 == PROP_RAW_GPU_ID {
                let ids = if (value >> 28) & 0xf == 0xf {
                    ((value >> 56) & 0xff, (value >> 32) & 0xff)
                } else {
                    ((value >> 28) & 0xf, (value >> 16) & 0xf)
                };
                if ids != G715 {
                    return Err(BenchError::Gpu(format!(
                        "gpu counters: GPU ID {value:#x} (arch {}, product {}) is not a \
                         Mali-G715, whose counter layout this reads",
                        ids.0, ids.1
                    )));
                }
                return Ok(());
            }
            rest = &rest[4 + width..];
        }
        Err(BenchError::Gpu(
            "gpu counters: the driver reported no raw GPU ID".into(),
        ))
    }

    /// The primary set's shader-core and L2 block counts, once the driver shows
    /// every block carries the indices read here.
    fn enumerate(fd: i32) -> Result<(u16, u16), BenchError> {
        let mut info = EnumInfo {
            item_size: 0,
            item_count: 0,
            list: 0,
        };
        // SAFETY: ENUM_INFO with a null list only reports its shape.
        unsafe { ioctl(fd, ENUM_INFO, &raw mut info, "enumerate counters")? };
        let stride = usize::try_from(info.item_size).expect("u32 fits usize");
        if stride < size_of::<EnumBlock>() {
            return Err(BenchError::Gpu(format!(
                "gpu counters: enumeration items of {stride} bytes are smaller than a block item"
            )));
        }
        let mut list =
            vec![0u8; stride * usize::try_from(info.item_count).expect("u32 fits usize")];
        info.list = list.as_mut_ptr() as u64;
        // SAFETY: the list holds `item_count` items of `item_size` bytes.
        unsafe { ioctl(fd, ENUM_INFO, &raw mut info, "enumerate counters")? };
        let mut shader_cores = None;
        let mut l2_slices = None;
        for item in list.chunks_exact(stride) {
            // SAFETY: each chunk is at least one `EnumBlock`, checked above.
            let block = unsafe { item.as_ptr().cast::<EnumBlock>().read_unaligned() };
            if block.header.kind != ENUM_BLOCK || block.instances == 0 {
                continue;
            }
            if block.set == SET_PRIMARY {
                match block.block {
                    BLOCK_SHADER_CORE if usize::from(block.values) > SC_CORE_ACTIVE => {
                        shader_cores = Some(block.instances);
                    }
                    BLOCK_MEMORY if usize::from(block.values) > L2_EXT_WRITE_BEATS => {
                        l2_slices = Some(block.instances);
                    }
                    _ => {}
                }
            }
        }
        shader_cores.zip(l2_slices).ok_or_else(|| {
            BenchError::Gpu(
                "gpu counters: the driver must enumerate primary shader-core and L2 blocks \
             with the requested counter indices"
                    .into(),
            )
        })
    }
}
