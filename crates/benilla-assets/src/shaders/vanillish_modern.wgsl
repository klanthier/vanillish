// Vanillish: the modern lighting look's shared maths (`benilla_assets::modern_light`), imported
// behind `VANILLISH_MODERN` by the world surface shaders: terrain, the model lane, static_gx and
// liquid. Pure functions with no bindings, so a pass with its own layout (static_gx) imports it
// too; each shader takes its own texture taps and hands the texels in.
//
// The frame's gamma contract holds: the reference's lighting laws are evaluated per pixel on a
// relief normal, in the same gamma-space values, and every extra is bounded: the cavity and the
// sheen sit inside the [0, 1] clamp the lit colour already had, and the water's sky lift mixes
// toward a sky colour, so no surface writes outside the range its stock shader writes, which is
// what the blends, the FFX glow and the UI expect.

#define_import_path benilla_assets::vanillish_modern

// `VANILLISH_RELIEF` and `VANILLISH_SHEEN`: percent of the tuned strengths below, 100 each.
const RELIEF: f32 = f32(#{VANILLISH_RELIEF}) / 100.0;
const SHEEN: f32 = f32(#{VANILLISH_SHEEN}) / 100.0;

// Normal tilt per unit of luminance change across one texel of the sampled mip: a 0.1 step tilts
// the normal by about 11°. Painted texels are noisy at that scale, so more reads as grain.
const BUMP: f32 = 2.0;
// Darkening per unit of the luminance Laplacian (a texel darker than its neighbours' mean), and
// the most it takes off: a crack 0.1 under its neighbours loses 15 %.
const CAVITY: f32 = 1.5;
const CAVITY_MAX: f32 = 0.25;

// The model lanes' sheen: its peak as a share of the sun colour, its Blinn exponent, and the warm
// tint that keeps it from reading as white plastic.
const MODEL_SHEEN: f32 = 0.3;
const MODEL_SHININESS: f32 = 24.0;
const WARM: vec3<f32> = vec3<f32>(1.0, 0.92, 0.78);
// Terrain's sheen keeps row 9 and the `_s` mask; per pixel its peak is resolved where Gouraud
// shading smeared it across 4-yard cells, so it is scaled down to the same overall weight.
const TERRAIN_SHEEN: f32 = 0.75;
// Water: how far a grazing view lifts the surface toward the sky colour and toward opaque.
const WATER_LIFT: f32 = 0.5;

// Rec. 709 luma of a gamma-space texel: the height the relief reads, bright being high.
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The screen-space derivatives the relief needs. Taken at the top of a fragment stage, before any
// branch or discard: implicit derivatives are undefined in non-uniform control flow, and a quad
// with a discarded lane has nothing to difference against. `p` should be camera-relative: an
// absolute world position interpolates values near 9000 yd, whose f32 steps would quantise them.
struct Derivs {
    dp_dx: vec3<f32>,
    dp_dy: vec3<f32>,
    duv_dx: vec2<f32>,
    duv_dy: vec2<f32>,
}

fn derivs(p: vec3<f32>, uv: vec2<f32>) -> Derivs {
    return Derivs(dpdx(p), dpdy(p), dpdx(uv), dpdy(uv));
}

// Where the four relief taps go, and the gradients that keep them on the main sample's mip: one
// texel of that mip out along u and along v (never under a full-size texel), sampled with the main
// sample's own derivatives scaled by its LOD bias, as `textureSampleBias` applies it. A tap with
// explicit gradients is valid in any control flow, and its offset follows the footprint smoothly,
// so the relief neither aliases nor shimmers as the mip changes.
struct Taps {
    off: vec2<f32>,
    ddx: vec2<f32>,
    ddy: vec2<f32>,
}

fn taps(d: Derivs, dims: vec2<f32>, mip_bias: f32) -> Taps {
    let scale = exp2(mip_bias);
    let ddx = d.duv_dx * scale;
    let ddy = d.duv_dy * scale;
    // Texels per pixel along the longer axis: the hardware LOD's 2^λ.
    let rho = max(length(ddx * dims), length(ddy * dims));
    return Taps(vec2<f32>(max(rho, 1.0)) / dims, ddx, ddy);
}

// The four tap heights (left, right, down, up). On a batch whose alpha is coverage (a cutout, a
// blend) a cut-out texel takes the centre's height `hc`: a leaf's edge is not a cliff.
fn heights(
    l: vec4<f32>,
    r: vec4<f32>,
    dn: vec4<f32>,
    up: vec4<f32>,
    hc: f32,
    coverage: bool,
) -> vec4<f32> {
    let h = vec4<f32>(luma(l.rgb), luma(r.rgb), luma(dn.rgb), luma(up.rgb));
    let cut = (vec4<f32>(l.a, r.a, dn.a, up.a) < vec4<f32>(0.5)) & vec4<bool>(coverage);
    return select(h, vec4<f32>(hc), cut);
}

// Schüler's cotangent frame ("Followup: Normal Mapping Without Precomputed Tangents", 2013): the
// world directions of increasing u and v in the plane of `n`, from the pixel's screen-space
// derivatives, so a mesh needs no tangent attribute. Scaled by the longer one's length, which keeps
// their ratio (a stretched texture's slopes stretch with it), and re-signed by the screen-space
// determinant, whose sign flips with the screen's y axis and with the side `n` faces: the bump then
// rises along `n` either way. (Mikkelsen's surface gradient is the same pair over |det|.)
struct Frame {
    t: vec3<f32>,
    b: vec3<f32>,
}

fn cotangent_frame(n: vec3<f32>, d: Derivs) -> Frame {
    let dp2perp = cross(d.dp_dy, n);
    let dp1perp = cross(n, d.dp_dx);
    let t = dp2perp * d.duv_dx.x + dp1perp * d.duv_dy.x;
    let b = dp2perp * d.duv_dx.y + dp1perp * d.duv_dy.y;
    let det = dot(d.dp_dx, dp2perp);
    let s = select(-1.0, 1.0, det >= 0.0) * inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-30));
    return Frame(t * s, b * s);
}

// The relief at one pixel.
struct Relief {
    // The tilted normal: unit, or zero where the surface normal is (an authored zero normal).
    n: vec3<f32>,
    // The cavity darkening, a factor in [1 − CAVITY_MAX, 1].
    cavity: f32,
}

// `n` the surface normal, `t`/`b` the frame of increasing u and v, `h` the tap heights against
// the centre's `hc`. The displaced surface's normal is N − ∇h along the surface, at a slope of
// BUMP per unit of luminance change across one sampled texel.
fn relief(n: vec3<f32>, t: vec3<f32>, b: vec3<f32>, h: vec4<f32>, hc: f32) -> Relief {
    let g = 0.5 * vec2<f32>(h.y - h.x, h.w - h.z);
    let p = n - (BUMP * RELIEF) * (g.x * t + g.y * b);
    let l2 = dot(p, p);
    let tilted = select(
        vec3<f32>(0.0),
        p * inverseSqrt(max(l2, 1e-24)),
        l2 > 1e-12 && dot(n, n) > 1e-12,
    );
    let lap = 0.25 * (h.x + h.y + h.z + h.w) - hc;
    let cavity = 1.0 - min(CAVITY * RELIEF * max(lap, 0.0), CAVITY_MAX);
    return Relief(tilted, cavity);
}

// The relief's absence: the normal as given, no cavity.
fn unrelieved(n: vec3<f32>) -> Relief {
    return Relief(n, 1.0);
}

// A soft Blinn-Phong sun sheen in [0, 1]: `n` the relief normal, `l` toward the sun, `v` toward
// the eye. It fades in over the terminator, so a face turned from the sun never glints.
fn sheen(n: vec3<f32>, l: vec3<f32>, v: vec3<f32>, shininess: f32) -> f32 {
    let hv = l + v;
    let h = hv * inverseSqrt(max(dot(hv, hv), 1e-12));
    let ndh = max(dot(n, h), 1e-4);
    return pow(ndh, shininess) * smoothstep(0.0, 0.2, dot(n, l));
}

// How glossy a texel reads: in hand-painted art the highlights (metal, glazed stone, wet leaves)
// are painted in, so the bright texels are the shiny ones.
fn gloss(lum: f32) -> f32 {
    return smoothstep(0.3, 0.9, lum);
}

// The model lanes' sheen colour for a sheen `s`: the sun's own (`Light.dbc`'s direct colour),
// warmed and part-tinted by the texel, so a highlight takes the paint's hue.
fn sheen_rgb(sun: vec3<f32>, albedo: vec3<f32>, s: f32) -> vec3<f32> {
    return sun * WARM * (0.35 + 0.65 * albedo) * (MODEL_SHEEN * SHEEN * s);
}

// One committed point light at `p` on the normal `n`: the reference's falloff
// `1/(0.7·d + 0.03·d²)`, diffuse only, as `point_light_sum` evaluates it per vertex.
fn point_light(p: vec3<f32>, n: vec3<f32>, pos: vec3<f32>, rgb: vec3<f32>) -> vec3<f32> {
    let to_light = pos - p;
    let d = length(to_light);
    let atten = 1.0 / max(0.7 * d + 0.03 * d * d, 1e-4);
    let nl = max(dot(n, to_light / max(d, 1e-4)), 0.0);
    return rgb * (atten * nl);
}

// Schlick's fresnel for water (F0 = 0.02): the share of the sky a surface reflects at this view.
fn fresnel(n: vec3<f32>, v: vec3<f32>) -> f32 {
    let c = 1.0 - clamp(dot(n, v), 0.0, 1.0);
    let c2 = c * c;
    return 0.02 + 0.98 * c2 * c2 * c;
}
