use wgpu::{Buffer, Device, ShaderModule};

pub struct ShaderView {
    buffer: Buffer,
}

impl ShaderView {
    // NOTE: The buffer usage should be configurable based on needs.
    // Using UNIFORM here as a common default.
    pub fn new(device: &Device, contents: &[u8]) -> Self {
        use wgpu::util::DeviceExt;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Shader View Buffer"),
            contents,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self { buffer }
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }
}

pub struct Shader {
    pub(crate) module: ShaderModule,
}

impl Shader {
    /// Creates a shader module from a WGSL source string.
    pub fn from_wgsl(device: &Device, source: &str) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        Self { module }
    }
}
