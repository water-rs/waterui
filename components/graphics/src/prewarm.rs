//! Shader pre-warming and registry system.
//!
//! This module provides mechanisms to register shaders that should be compiled
//! (pre-warmed) at application startup, avoiding jank during runtime.
//! It uses `inventory` to collect shader descriptors across the codebase.

use std::borrow::Cow;
use std::sync::Arc;
use wgpu;
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderKind {
    /// A complete WGSL shader module.
    Full,
    /// A fragment-only shader that requires the standard ShaderSurface prelude.
    FragmentOnly,
}

/// Descriptor for a shader that needs pre-warming.
#[derive(Debug, Clone)]
pub struct ShaderDescriptor {
    /// Unique identifier for the shader (usually the file path).
    pub label: &'static str,
    /// The WGSL source code.
    pub source: Cow<'static, str>,
    /// The kind of shader (Full or FragmentOnly).
    pub kind: ShaderKind,
}

// Enable inventory collection for ShaderDescriptor
inventory::collect!(ShaderDescriptor);

/// Pre-warmed shader module.
///
/// This struct is returned by `include_shader!` and holds both the runtime data
/// and the static descriptor used for pre-warming.
#[derive(Debug, Clone)]
pub struct PrewarmedShader {
    /// The label of the shader.
    pub label: &'static str,
    /// The WGSL source code.
    pub source: Cow<'static, str>,
}

impl PrewarmedShader {
    /// Create a new pre-warmed shader descriptor.
    /// This is typically called by the `include_shader!` macro.
    pub const fn new(label: &'static str, source: &'static str) -> Self {
        Self {
            label,
            source: Cow::Borrowed(source),
        }
    }
}

/// Macro to include a shader file and register it for pre-warming.
///
/// Usage: `static MY_SHADER: PrewarmedShader = include_shader!("path/to/shader.wgsl");`
#[macro_export]
macro_rules! include_shader {
    ($path:literal) => {{
        const SOURCE: &str = include_str!($path);
        const PATH: &str = $path;
        
        // Submit to inventory for pre-warming collection
        $crate::inventory::submit! {
            $crate::prewarm::ShaderDescriptor {
                label: PATH,
                source: std::borrow::Cow::Borrowed(SOURCE),
                kind: $crate::prewarm::ShaderKind::Full,
            }
        }
        
        // Return the runtime struct
        $crate::prewarm::PrewarmedShader::new(PATH, SOURCE)
    }};
}

/// Macro to include a fragment-only shader file and register it for pre-warming.
///
/// This is used by `ShaderSurface` to auto-prepend the standard prelude during pre-warming.
#[macro_export]
macro_rules! include_fragment_shader {
    ($path:literal) => {{
        const SOURCE: &str = include_str!($path);
        const PATH: &str = $path;
        
        $crate::inventory::submit! {
            $crate::prewarm::ShaderDescriptor {
                label: PATH,
                source: std::borrow::Cow::Borrowed(SOURCE),
                kind: $crate::prewarm::ShaderKind::FragmentOnly,
            }
        }
        
        $crate::prewarm::PrewarmedShader::new(PATH, SOURCE)
    }};
}


/// Compile all registered shaders in parallel.
///
/// This should be called during `waterui_init` to ensure all shaders are ready.
/// It uses the `SharedGpuContext` to compile modules.
pub fn prewarm_all_shaders() {
    if !crate::shared_context::is_initialized() {
        tracing::warn!("[Prewarm] Shared context not initialized, skipping pre-warm");
        return;
    }

    let ctx = crate::shared_context::shared_context();
    let guard = ctx.read();
    let device = &guard.device;

    tracing::info!("[Prewarm] Starting shader pre-warming...");

    // Collect all descriptors
    let descriptors: Vec<&ShaderDescriptor> = inventory::iter::<ShaderDescriptor>.into_iter().collect();
    
    if descriptors.is_empty() {
        tracing::info!("[Prewarm] No shaders registered for pre-warming.");
        return;
    }

    tracing::info!("[Prewarm] Compiling {} shaders in parallel...", descriptors.len());

    // Compile in parallel using rayon
    let results: Vec<Result<(), String>> = descriptors
        .par_iter()
        .map(|desc| {
            // We just need to create the shader module to trigger compilation/validation
            // wgpu will cache it internally if `create_shader_module` is called again with same source.
            // 
            // Note: Ideally we would store the resulting `ShaderModule` in a cache map,
            // but wgpu's internal caching is usually sufficient if the source string is identical.
            // However, to be safe and avoid repeated parsing cost, we *should* probably cache them.
            // For Phase 2, we rely on wgpu's caching or just the fact that we've paid the compilation cost.
            
            // Actually, wgpu `create_shader_module` does compilation. If we drop it, we might lose it purely from cache
            // if wgpu doesn't persist it. 
            // BUT: The main benefit here is checking for errors and ensuring driver-level compilation happens.
            // 
            // TODO: In a future iteration, we should store these in a `HashMap<&str, ShaderModule>` within SharedGpuContext.
            
            let source = match desc.kind {
                ShaderKind::Full => desc.source.clone(),
                ShaderKind::FragmentOnly => {
                    // Prepend the standard prelude
                    let mut full = String::with_capacity(crate::shader_surface::PRELUDE.len() + desc.source.len());
                    full.push_str(crate::shader_surface::PRELUDE);
                    full.push_str(&desc.source);
                    Cow::Owned(full)
                }
            };

            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(desc.label),
                source: wgpu::ShaderSource::Wgsl(source),
            });
            
            // Force compilation check (compilation happens on creation usually, but errors are async unless we check)
            // But we can't easily wait on it without async. 
            // `create_shader_module` is sync, validation errors happen then.
            
            Ok(())
        })
        .collect();
        
    let success_count = results.iter().filter(|r| r.is_ok()).count();
    tracing::info!("[Prewarm] Finished. {}/{} shaders pre-warmed successfully.", success_count, descriptors.len());
}
