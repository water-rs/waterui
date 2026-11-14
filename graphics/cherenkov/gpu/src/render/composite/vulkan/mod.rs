//! On-chip composition on Vulkan (issue #2191): what a device must have
//! enabled to keep a tile epoch's intermediates in tile memory, and the
//! ordering those capabilities guarantee.
//!
//! wgpu exposes neither framebuffer fetch nor input attachments
//! (gfx-rs/wgpu#10148), so the epoch itself is recorded natively through
//! `as_hal` on the engine's own device and queue. The capabilities that
//! record needs are not an `as_hal` decision: an extension and its
//! feature bits are chosen when `vkCreateDevice` runs, and nothing can
//! turn them on afterwards. This module is therefore read twice — once
//! by [`crate::render::create_vulkan_device`], which asks for them, and
//! once by the engine, which reads back what the device it was handed
//! actually enabled.
//!
//! Three capabilities matter, and they compose:
//!
//! - `VK_KHR_dynamic_rendering` (core in Vulkan 1.3, and a feature bit
//!   wgpu never enables because wgpu-hal records render pass objects):
//!   a native epoch is one `vkCmdBeginRendering` scope, so that the
//!   engine's attachments need no `VkFramebuffer` of their own;
//! - `VK_KHR_dynamic_rendering_local_read`: a colour attachment of the
//!   current rendering scope can be read as an input attachment, which
//!   is how a draw reads the destination or an earlier slot without a
//!   round trip to memory. Its per-draw index and location remapping
//!   (`vkCmdSetRenderingInputAttachmentIndices`,
//!   `vkCmdSetRenderingAttachmentLocations`) is what lets one shader
//!   entry point serve every (target, source) slot pair;
//! - `VK_EXT_rasterization_order_attachment_access`, colour aspect: two
//!   fragments covering one pixel are ordered against each other, so a
//!   draw reads exactly what the epoch's earlier draws wrote — the
//!   guarantee Apple's `raster_order_group(0)` gives. Without it, reads
//!   are ordered only between draws separated by a barrier.
//!
//! `VK_EXT_shader_tile_image` is the floor's second per-pixel mechanism.
//! It is not requested here: it reads attachments through a different
//! shader interface (`OpColorAttachmentReadEXT` rather than an input
//! attachment), so it needs its own SPIR-V, and every device of the
//! floor that has it — Mali-G715, Adreno 830/840 — also has
//! rasterization-order attachment access. A device that had only tile
//! image would get [`Access::None`] and compose portably.
//!
//! Access alone does not make a device eligible. An epoch's slots exist
//! to keep intermediates out of device memory (#2192), so they are
//! backed by lazily allocated memory, which a tile GPU leaves
//! uncommitted while the slot stays on chip. A device with per-pixel
//! access and no lazily allocated memory type — a software rasterizer
//! such as llvmpipe — has no tile memory for an epoch to stay in:
//! backing its slots with ordinary device memory would add an
//! allocation per slot on top of the targets the portable path already
//! holds, for no traffic saved. [`eligibility`] decides from both, once,
//! when the engine is created, and such a device composes portably.

pub mod exec;
pub mod mirror;

use std::ffi::CStr;

use ash::vk;

pub use mirror::{Mirror, SLOTS};

/// What a Vulkan device's *enabled* capabilities let a tile epoch do.
///
/// This is read off the device, never off the adapter: an adapter that
/// supports an extension says nothing about a device that was opened
/// without it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// No colour attachment of a rendering scope can be read back, so
    /// no epoch can form and every logical image is materialized.
    None,
    /// A draw can read a slot an *earlier draw* wrote, as an input
    /// attachment, when an explicit by-region barrier separates them.
    /// Two fragments of one draw covering one pixel are unordered.
    PerDraw,
    /// A draw reads exactly what the epoch's earlier fragments wrote to
    /// the same pixel, with no barrier at all.
    PerPixel,
}

/// The extensions a rendering scope's local reads need, in the order
/// they build on each other.
///
/// `VK_KHR_dynamic_rendering` is listed even on a Vulkan 1.3 device
/// where it is core: a promoted extension is accepted in
/// `ppEnabledExtensionNames` only when the physical device still
/// reports it, so [`Request::extensions`] filters this list against the
/// device's own and the promoted case simply drops out of it. The
/// `dynamicRendering` feature bit is enabled either way.
const BASE_NAMES: [&CStr; 2] = [
    ash::khr::dynamic_rendering::NAME,
    ash::khr::dynamic_rendering_local_read::NAME,
];

/// The two names ordered attachment access is published under, newest
/// first.
///
/// `VK_EXT_rasterization_order_attachment_access` is the promotion of
/// `VK_ARM_rasterization_order_attachment_access`, sharing its feature
/// struct and structure type verbatim, and a Mali driver may still
/// publish only the ARM name. Both are the same capability; which name
/// the device answers to is not a capability difference, so the
/// supported one is the one asked for.
const ORDERED_NAMES: [&CStr; 2] = [
    vk::EXT_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_NAME,
    vk::ARM_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_NAME,
];

/// Whichever name of [`ORDERED_NAMES`] `supported` admits.
fn ordered_name(supported: impl Fn(&CStr) -> bool) -> Option<&'static CStr> {
    ORDERED_NAMES.into_iter().find(|&name| supported(name))
}

/// The access `enabled` — a device's enabled extension list — allows.
///
/// A device that enabled one of these extensions was opened through
/// [`crate::render::create_vulkan_device`], the only place in the engine
/// or its hosts that asks for one, and which enables each extension's
/// feature bits with it. Extension presence is therefore feature
/// presence; nothing else can report this, because Vulkan offers no way
/// to ask a device which features it enabled.
pub fn access(enabled: &[&'static CStr]) -> Access {
    let has = |name: &CStr| enabled.contains(&name);
    if !has(ash::khr::dynamic_rendering_local_read::NAME) {
        return Access::None;
    }
    if ordered_name(has).is_some() {
        Access::PerPixel
    } else {
        Access::PerDraw
    }
}

/// The access `device` was opened with, or [`Access::None`] when it is
/// not a Vulkan device at all.
pub fn device_access(device: &wgpu::Device) -> Access {
    // SAFETY: `device` is borrowed for this scope and only its enabled
    // extension list is read; `as_hal` reports `None` off-Vulkan.
    let Some(hal) = (unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() }) else {
        return Access::None;
    };
    access(hal.enabled_device_extensions())
}

/// Why a device cannot run a tile epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declined {
    /// The device was opened with less than per-pixel attachment
    /// access.
    Access(Access),
    /// The device has per-pixel access but no lazily allocated memory
    /// type, so an epoch's slots would have no tile memory to stay in.
    TransientMemory,
}

/// A device that can run a tile epoch: it was opened with per-pixel
/// attachment access, and its memory has a lazily allocated type for
/// the epoch's slots.
///
/// Only [`eligibility`] makes one, and [`Mirror::new`] takes it, so no
/// executor is built for a device that cannot run it.
#[derive(Clone, Copy, Debug)]
pub struct Eligible {
    /// Bit `i` is set when memory type `i` is lazily allocated — the
    /// same encoding as an image's `memoryTypeBits`.
    lazy_types: u32,
}

impl Eligible {
    /// The first lazily allocated memory type among `bits`, an image's
    /// `memoryTypeBits`.
    #[must_use]
    pub const fn lazy_memory_type(self, bits: u32) -> Option<u32> {
        match self.lazy_types & bits {
            0 => None,
            admitted => Some(admitted.trailing_zeros()),
        }
    }
}

/// Whether a device opened with `access`, whose physical device reports
/// `memory`, can run a tile epoch.
///
/// # Errors
/// [`Declined`] names the capability the device lacks, access first.
pub fn eligibility(
    access: Access,
    memory: &vk::PhysicalDeviceMemoryProperties,
) -> Result<Eligible, Declined> {
    if access != Access::PerPixel {
        return Err(Declined::Access(access));
    }
    let count = usize::try_from(memory.memory_type_count).expect("memory type count fits usize");
    let lazy_types = memory.memory_types[..count]
        .iter()
        .enumerate()
        .filter(|(_, ty)| {
            ty.property_flags
                .contains(vk::MemoryPropertyFlags::LAZILY_ALLOCATED)
        })
        .fold(0, |mask, (index, _)| mask | (1 << index));
    if lazy_types == 0 {
        return Err(Declined::TransientMemory);
    }
    Ok(Eligible { lazy_types })
}

/// [`eligibility`] for the engine's own `device`, opened on `adapter`;
/// a device that is not Vulkan declines with [`Access::None`].
///
/// # Errors
/// [`Declined`] names the capability the device lacks.
pub fn device_eligibility(
    device: &wgpu::Device,
    adapter: &wgpu::Adapter,
) -> Result<Eligible, Declined> {
    let access = device_access(device);
    // SAFETY: `as_hal` borrows the adapter for this scope and reports
    // `None` off-Vulkan.
    let Some(hal) = (unsafe { adapter.as_hal::<wgpu::hal::vulkan::Api>() }) else {
        return Err(Declined::Access(Access::None));
    };
    // SAFETY: the physical device belongs to the live instance `hal`
    // shares; the query writes only its own return value.
    let memory = unsafe {
        hal.shared_instance()
            .raw_instance()
            .get_physical_device_memory_properties(hal.raw_physical_device())
    };
    eligibility(access, &memory)
}

/// The feature bits that go with [`device_extensions`], as one `p_next`
/// chain to splice into a `VkDeviceCreateInfo`.
///
/// Each struct is boxed so its address stays valid while the chain is
/// linked and `vkCreateDevice` reads it; a `Request` therefore has to
/// outlive the call it is spliced into.
pub struct Request {
    dynamic_rendering: Box<vk::PhysicalDeviceDynamicRenderingFeatures<'static>>,
    local_read: Box<vk::PhysicalDeviceDynamicRenderingLocalReadFeaturesKHR<'static>>,
    rasterization_order:
        Box<vk::PhysicalDeviceRasterizationOrderAttachmentAccessFeaturesEXT<'static>>,
    access: Access,
}

impl Request {
    /// What `physical` can actually give, queried through
    /// `vkGetPhysicalDeviceFeatures2`: an extension in the device's list
    /// still leaves its features optional, and
    /// `rasterizationOrderColorAttachmentAccess` is the one of the three
    /// that a device may report without.
    ///
    /// # Safety
    /// `instance` must own `physical`, and both must outlive this call.
    pub unsafe fn probe(
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
        supported: impl Fn(&CStr) -> bool,
    ) -> Self {
        let mut dynamic_rendering = Box::new(vk::PhysicalDeviceDynamicRenderingFeatures::default());
        let mut local_read =
            Box::new(vk::PhysicalDeviceDynamicRenderingLocalReadFeaturesKHR::default());
        let mut rasterization_order =
            Box::new(vk::PhysicalDeviceRasterizationOrderAttachmentAccessFeaturesEXT::default());
        let mut features = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut *dynamic_rendering)
            .push_next(&mut *local_read)
            .push_next(&mut *rasterization_order);
        // SAFETY: the caller's contract — `physical` belongs to
        // `instance`, and every struct in the chain lives in this scope.
        unsafe { instance.get_physical_device_features2(physical, &mut features) };
        // A promoted extension need not appear in the device's list; the
        // feature bit is what the 1.3 device answers on.
        let rendering = dynamic_rendering.dynamic_rendering == vk::TRUE;
        let read = rendering
            && local_read.dynamic_rendering_local_read == vk::TRUE
            && supported(ash::khr::dynamic_rendering_local_read::NAME);
        let ordered = read
            && rasterization_order.rasterization_order_color_attachment_access == vk::TRUE
            && ordered_name(&supported).is_some();
        let access = if ordered {
            Access::PerPixel
        } else if read {
            Access::PerDraw
        } else {
            Access::None
        };
        // Only what this device will be opened with stays set: the
        // request is the chain handed to `vkCreateDevice`, and a feature
        // the engine does not get must not be asked for.
        *dynamic_rendering = vk::PhysicalDeviceDynamicRenderingFeatures::default()
            .dynamic_rendering(access != Access::None);
        *local_read = vk::PhysicalDeviceDynamicRenderingLocalReadFeaturesKHR::default()
            .dynamic_rendering_local_read(access != Access::None);
        *rasterization_order =
            vk::PhysicalDeviceRasterizationOrderAttachmentAccessFeaturesEXT::default()
                .rasterization_order_color_attachment_access(access == Access::PerPixel);
        Self {
            dynamic_rendering,
            local_read,
            rasterization_order,
            access,
        }
    }

    /// The access a device opened with this request will report.
    pub const fn access(&self) -> Access {
        self.access
    }

    /// The extensions this request needs, as far as `supported` admits
    /// them: nothing at all when the device cannot compose on chip.
    pub fn extensions(&self, supported: impl Fn(&CStr) -> bool) -> Vec<&'static CStr> {
        if self.access == Access::None {
            return Vec::new();
        }
        let mut names: Vec<&'static CStr> = BASE_NAMES
            .into_iter()
            .filter(|&name| supported(name))
            .collect();
        if self.access == Access::PerPixel {
            names.extend(ordered_name(&supported));
        }
        names
    }

    /// Links the request's structs into one chain whose tail is `tail` —
    /// the `p_next` the create info already carried — and answers the
    /// new head. `None` when the device composes portably and the chain
    /// is empty.
    ///
    /// Only the structs of extensions the device is actually opened with
    /// are linked: a feature struct of an extension that is not enabled
    /// is invalid usage, so a `PerDraw` device carries no
    /// rasterization-order struct at all rather than one asking for
    /// nothing.
    ///
    /// # Safety
    /// The returned pointer borrows `self`, which must outlive the
    /// `vkCreateDevice` call it is handed to, and `tail` must be either
    /// null or a `p_next` chain that outlives it too.
    pub unsafe fn chain(
        &mut self,
        tail: *const core::ffi::c_void,
    ) -> Option<*const core::ffi::c_void> {
        if self.access == Access::None {
            return None;
        }
        let mut head = tail.cast_mut();
        if self.access == Access::PerPixel {
            self.rasterization_order.p_next = head;
            head = core::ptr::from_mut(&mut *self.rasterization_order).cast();
        }
        self.local_read.p_next = head;
        self.dynamic_rendering.p_next = core::ptr::from_mut(&mut *self.local_read).cast();
        Some(core::ptr::from_ref(&*self.dynamic_rendering).cast())
    }
}

#[cfg(test)]
mod tests {
    use ash::vk;

    use super::{Access, BASE_NAMES, Declined, ORDERED_NAMES, access, eligibility};

    /// Memory properties listing one type per entry of `types`.
    fn memory(types: &[vk::MemoryPropertyFlags]) -> vk::PhysicalDeviceMemoryProperties {
        let mut memory = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: u32::try_from(types.len()).expect("test lists few types"),
            ..Default::default()
        };
        for (slot, &property_flags) in memory.memory_types.iter_mut().zip(types) {
            *slot = vk::MemoryType {
                property_flags,
                heap_index: 0,
            };
        }
        memory
    }

    const LAZY: vk::MemoryPropertyFlags = vk::MemoryPropertyFlags::from_raw(
        vk::MemoryPropertyFlags::DEVICE_LOCAL.as_raw()
            | vk::MemoryPropertyFlags::LAZILY_ALLOCATED.as_raw(),
    );

    #[test]
    fn per_pixel_access_with_lazy_memory_is_eligible() {
        let eligible = eligibility(
            Access::PerPixel,
            &memory(&[vk::MemoryPropertyFlags::DEVICE_LOCAL, LAZY]),
        )
        .expect("a tile GPU runs epochs");
        assert_eq!(eligible.lazy_memory_type(0b11), Some(1));
        assert_eq!(eligible.lazy_memory_type(0b01), None);
    }

    /// llvmpipe: per-pixel access, and every memory type committed
    /// up front (#2689).
    #[test]
    fn per_pixel_access_without_lazy_memory_declines() {
        let host = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        assert_eq!(
            eligibility(
                Access::PerPixel,
                &memory(&[vk::MemoryPropertyFlags::DEVICE_LOCAL | host]),
            )
            .err(),
            Some(Declined::TransientMemory)
        );
    }

    #[test]
    fn weaker_access_declines_before_memory_is_read() {
        for weaker in [Access::None, Access::PerDraw] {
            assert_eq!(
                eligibility(weaker, &memory(&[LAZY])).err(),
                Some(Declined::Access(weaker))
            );
        }
    }

    #[test]
    fn local_read_alone_orders_only_between_draws() {
        assert_eq!(access(&BASE_NAMES), Access::PerDraw);
    }

    #[test]
    fn ordered_access_without_local_read_reads_nothing() {
        assert_eq!(access(&ORDERED_NAMES), Access::None);
        assert_eq!(access(&[]), Access::None);
    }

    #[test]
    fn either_ordered_name_orders_per_pixel() {
        for ordered in ORDERED_NAMES {
            let enabled = [BASE_NAMES[0], BASE_NAMES[1], ordered];
            assert_eq!(access(&enabled), Access::PerPixel);
        }
    }
}
