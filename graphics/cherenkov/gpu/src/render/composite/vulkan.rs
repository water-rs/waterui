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

use std::ffi::CStr;

use ash::vk;

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
    use super::{Access, BASE_NAMES, ORDERED_NAMES, access};

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
