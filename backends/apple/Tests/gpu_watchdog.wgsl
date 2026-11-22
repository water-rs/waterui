// A compute dispatch long enough that the GPU watchdog terminates its
// command buffer: each invocation runs five million xorshift iterations.
// The same workload as wgpu's `WAIT_INDEFINITELY_LONG_RUNNING` test.
@group(0) @binding(0) var<storage, read_write> buf: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    var x = gid.x + 1u;
    for (var i = 0u; i < 5000000u; i = i + 1u) {
        x ^= x << 13u;
        x ^= x >> 17u;
        x ^= x << 5u;
    }
    buf[gid.x] = x;
}
