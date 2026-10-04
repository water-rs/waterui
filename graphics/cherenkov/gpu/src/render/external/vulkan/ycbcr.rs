//! `VkSamplerYcbcrConversion` objects, their immutable samplers and the
//! combined-image-sampler set-1 layouts they imply.
//!
//! Conversion/layout objects are cached on the shared context by the
//! complete conversion key — external-format identity (or concrete
//! format), model, range, component mapping, chroma locations and
//! filtering — never just by extent. The cache is bounded; a full cache is
//! an `Unsupported` error rather than an unbounded native allocation.

use std::sync::{Arc, Weak};

use ash::vk;

use super::{NativeError, Shared, Vk};

/// The maximum live conversion objects cached per device.
const CONV_CACHE_MAX: usize = 64;

/// The complete conversion contract a `VkSamplerYcbcrConversion` implements.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConvKey {
    /// `vk::Format` for known-format conversions, `UNDEFINED` for external.
    pub format: vk::Format,
    /// The `externalFormat` identifier; `0` for ordinary formats.
    pub external_format: u64,
    pub model: vk::SamplerYcbcrModelConversion,
    pub range: vk::SamplerYcbcrRange,
    /// Component mapping as `r, g, b, a` — `vk::ComponentMapping` carries
    /// no `Eq`/`Hash` for use as a key.
    pub mapping: [vk::ComponentSwizzle; 4],
    pub chroma_x: vk::ChromaLocation,
    pub chroma_y: vk::ChromaLocation,
    pub filter: vk::Filter,
}

/// A `VkSamplerYcbcrConversion`, the immutable `VkSampler` carrying it, and
/// the set-1 layout whose combined-image-sampler binding the sampler is
/// baked into.
#[derive(Debug)]
pub struct Conv {
    /// The conversion's full key, kept so `Weak`-expired cache slots can
    /// be re-keyed and so `destroy` can drop the layout it created.
    #[allow(dead_code)]
    pub key: ConvKey,
    /// The `VkSamplerYcbcrConversion`.
    pub conversion: vk::SamplerYcbcrConversion,
    /// The immutable sampler bound inside the descriptor layout.
    pub sampler: vk::Sampler,
    /// Group-1 layout: the mask texture (3), the params uniform (4) and the
    /// combined sampled image at binding 5 carrying this conversion.
    pub set1: vk::DescriptorSetLayout,
}

impl Conv {
    /// Creates the conversion, the immutable sampler and the layout for
    /// `key`. Everything here is device-level; the pipeline layout that
    /// embeds the layout is cached per-renderer.
    pub fn new(shared: &Shared, key: &ConvKey) -> Result<Self, NativeError> {
        let vk_ctx = &shared.vk;
        let Some(ycbcr) = &vk_ctx.ycbcr else {
            return Err(NativeError::Unsupported(
                "sampler_ycbcr_conversion is not enabled",
            ));
        };
        let mut external;
        let mut info = vk::SamplerYcbcrConversionCreateInfo::default()
            .format(key.format)
            .ycbcr_model(key.model)
            .ycbcr_range(key.range)
            .components(vk::ComponentMapping {
                r: key.mapping[0],
                g: key.mapping[1],
                b: key.mapping[2],
                a: key.mapping[3],
            })
            .x_chroma_offset(key.chroma_x)
            .y_chroma_offset(key.chroma_y)
            .chroma_filter(key.filter);
        if key.format == vk::Format::UNDEFINED {
            external = vk::ExternalFormatANDROID::default().external_format(key.external_format);
            info = info.push_next(&mut external);
        }
        // SAFETY: `ycbcr`'s device is live and `info` points at the
        // conversion parameters built above (including the external-format
        // chain when present).
        let conversion = unsafe { ycbcr.create_sampler_ycbcr_conversion(&info, None) }
            .map_err(NativeError::from)?;

        let mut conv_info = vk::SamplerYcbcrConversionInfo::default().conversion(conversion);
        // SAFETY: `vk_ctx.device` is live and the create info chains
        // `conv_info`, which points at the conversion created above.
        let sampler = unsafe {
            vk_ctx.device.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(key.filter)
                    .min_filter(key.filter)
                    .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .push_next(&mut conv_info),
                None,
            )
        }
        .map_err(NativeError::from)?;

        let fs = vk::ShaderStageFlags::FRAGMENT;
        let immutable = [sampler];
        // SAFETY: `vk_ctx.device` is live; the bindings are compile-time
        // and `immutable` references the sampler created above.
        let set1 = unsafe {
            vk_ctx.device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&[
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(3)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .descriptor_count(1)
                        .stage_flags(fs),
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(4)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .descriptor_count(1)
                        .stage_flags(fs),
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(5)
                        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                        .descriptor_count(1)
                        .stage_flags(fs)
                        .immutable_samplers(&immutable),
                ]),
                None,
            )
        };
        match set1 {
            Ok(set1) => Ok(Self {
                key: key.clone(),
                conversion,
                sampler,
                set1,
            }),
            Err(err) => {
                // SAFETY: `sampler` and `conversion` were created above
                // on this device and are destroyed exactly once on this
                // error path.
                unsafe {
                    vk_ctx.device.destroy_sampler(sampler, None);
                    ycbcr.destroy_sampler_ycbcr_conversion(conversion, None);
                }
                Err(err.into())
            }
        }
    }

    /// Destroys in dependency order: set layout, sampler, conversion.
    pub fn destroy(self, vk_ctx: &Vk) {
        let dev = &vk_ctx.device;
        // SAFETY: `self` is consumed, so each object — created on `dev`
        // by `open` — is destroyed exactly once, and no pending work
        // references them (the caller releases only after the release
        // submission).
        unsafe {
            dev.destroy_descriptor_set_layout(self.set1, None);
            dev.destroy_sampler(self.sampler, None);
            if let Some(ycbcr) = &vk_ctx.ycbcr {
                ycbcr.destroy_sampler_ycbcr_conversion(self.conversion, None);
            }
        }
    }
}

/// The cached-or-fresh `Conv` for `key`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[allow(clippy::significant_drop_tightening)]
pub fn get(shared: &Arc<Shared>, key: ConvKey) -> Result<Arc<Conv>, NativeError> {
    let mut cache = shared.convs.lock().expect("conversion cache");
    if let Some(conv) = cache.get(&key).and_then(Weak::upgrade) {
        return Ok(conv);
    }
    // Reap dead entries before checking the bound.
    cache.retain(|_, weak| weak.strong_count() > 0);
    if cache.len() >= CONV_CACHE_MAX {
        return Err(NativeError::Unsupported("conversion cache is full"));
    }
    let conv = Arc::new(Conv::new(shared, &key)?);
    cache.insert(key, Arc::downgrade(&conv));
    Ok(conv)
}
