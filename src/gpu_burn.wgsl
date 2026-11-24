struct Params {
    rounds: u32,
    elements: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0)
var<uniform> params: Params;

@group(0) @binding(1)
var<storage, read_write> state: array<u32>;

@compute @workgroup_size(256)
fn burn(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.elements) {
        return;
    }

    var value = state[index] ^ (index * 747796405u + 2891336453u);
    if (value == 0u) {
        value = 1u;
    }

    for (var round = 0u; round < params.rounds; round += 1u) {
        value ^= value << 13u;
        value ^= value >> 17u;
        value ^= value << 5u;
        value = value * 1664525u + 1013904223u;
    }

    state[index] = value;
}
