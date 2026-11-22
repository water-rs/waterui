//! The reference interpreter's module-level surface: bound globals, entry
//! points and host-bound functions.

#![cfg(feature = "eval")]

use cherenkov_shader::eval::{Eval, Value, assert_close};
use cherenkov_shader::naga;

fn module(source: &str) -> naga::Module {
    naga::front::wgsl::parse_str(source).expect("the test shader parses")
}

#[test]
fn an_entry_point_reads_bound_uniforms_by_constant_and_dynamic_index() {
    let module = module(
        "struct Uniforms { time: f32, scale: f32 }
         @group(0) @binding(0) var<uniform> uniforms: Uniforms;
         @group(0) @binding(1) var<uniform> params: array<vec4<f32>, 4>;
         @fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
             var total = vec4<f32>(0.0);
             for (var i = 0; i < 4; i = i + 1) {
                 total += params[i];
             }
             return total * uniforms.scale + vec4<f32>(uv, params[1].x, 0.0);
         }",
    );
    let mut eval = Eval::new(&module);
    eval.bind(
        "uniforms",
        Value::Struct(vec![Value::Float(0.0), Value::Float(2.0)]),
    );
    eval.bind(
        "params",
        Value::Struct(vec![
            Value::vec(&[1.0, 0.0, 0.0, 0.0]),
            Value::vec(&[0.25, 1.0, 0.0, 0.0]),
            Value::vec(&[0.0, 0.0, 1.0, 0.0]),
            Value::vec(&[0.0, 0.0, 0.0, 1.0]),
        ]),
    );
    let result = eval.call_entry("main", vec![Value::vec(&[0.5, 0.75])]);
    assert_close(&result, &Value::vec(&[3.0, 2.75, 2.25, 2.0]), 1e-6);
}

#[test]
fn a_host_function_replaces_a_declared_hook() {
    let module = module(
        "fn hook(q: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }
         fn effect(p: vec2<f32>) -> vec4<f32> {
             return hook(p + vec2<f32>(1.0, 0.0)) * 0.5;
         }",
    );
    let mut eval = Eval::new(&module);
    eval.host("hook", |args| {
        let q = args[0].components();
        Value::vec(&[q[0], q[1], 1.0, 1.0])
    });
    let effect = eval.function("effect");
    let result = eval.call(effect, vec![Value::vec(&[2.0, 3.0])]);
    assert_close(&result, &Value::vec(&[1.5, 1.5, 0.5, 0.5]), 1e-6);
}

#[test]
fn smoothstep_matches_its_definition() {
    let module = module("fn f(x: f32) -> f32 { return smoothstep(0.25, 0.75, x); }");
    let eval = Eval::new(&module);
    let f = eval.function("f");
    for (x, expected) in [(0.0, 0.0), (0.5, 0.5), (0.375, 0.156_25), (1.0, 1.0)] {
        assert_close(
            &eval.call(f, vec![Value::Float(x)]),
            &Value::Float(expected),
            1e-6,
        );
    }
}

#[test]
#[should_panic(expected = "global `params` read before it was bound")]
fn an_unbound_global_fails_by_name() {
    let module = module(
        "@group(0) @binding(0) var<uniform> params: vec4<f32>;
         @fragment fn main() -> @location(0) vec4<f32> { return params; }",
    );
    let _ = Eval::new(&module).call_entry("main", Vec::new());
}
