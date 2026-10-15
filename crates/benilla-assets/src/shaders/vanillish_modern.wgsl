// Vanillish: the modern lighting look's shared maths (`benilla_assets::modern_light`), imported
// behind `VANILLISH_MODERN` by the world surface shaders: terrain, the model lane, static_gx and
// liquid. Pure functions with no bindings, so a pass with its own layout (static_gx) imports it
// too; each shader takes its own texture taps and hands the texels in.
//
// The stock lighting is the baseline, computed as the stock shader computes it, and the relief
// moves the light around it, never its average: every relief term is linear in the tap heights
// (a slope, a Laplacian), so a bump and its mirror image cancel and a surface keeps its stock
// brightness, glare and colour, with cracks, joints and seams reading as 3D under the sun.
// `modern_light::relief_tests` checks this maths on the host against a Rust twin.
//
// The frame's gamma contract holds: everything is in the same gamma-space values, and every
// extra stays inside the [0, 1] clamp the lit colour already had (the water's sky lift mixes
// toward a sky colour), so no surface writes outside the range its stock shader writes, which is
// what the blends, the FFX glow and the UI expect.

#define_import_path benilla_assets::vanillish_modern

// `VANILLISH_RELIEF` and `VANILLISH_SHEEN`: percent of the tuned strengths below, 100 each.
const RELIEF: f32 = f32(#{VANILLISH_RELIEF}) / 100.0;
const SHEEN: f32 = f32(#{VANILLISH_SHEEN}) / 100.0;

// Normal tilt per unit of luminance change across one texel of the sampled mip: a 0.1 step tilts
// the normal by about 11°. It sets the slope every relief term reads.
const BUMP: f32 = 2.0;
// The share of the slope's change to a Lambert term N·L that the light sum takes. With BUMP, a
// strong texel edge (a 0.25 luminance step between neighbouring texels: a crack, a mortar joint, a
// plank seam) facing a sun 30° up moves a typical lit diffuse (ambient 0.4, sun 0.7) by about
// ±30 %, ±20 % when it runs diagonal to the sun, while paint noise (0.04 steps) stays under ±5 %:
// relief that reads at play distance without grain.
const RELIEF_GAIN: f32 = 1.5;
// The cavity: brightness per unit of the luminance Laplacian (the neighbours' mean less the
// centre), two-sided and capped, so a crack 0.1 under its neighbours loses 12.5 %, a ridge as
// far over gains as much, and over a whole texture the factors average to one. A Laplacian
// sharpens, so a stronger one turns paint noise into grain.
const CAVITY: f32 = 1.25;
const CAVITY_MAX: f32 = 0.15;
// The glare's sparkle: the stock specular's scale per unit of the slope along the half vector,
// and its bounds. Where row 9's glare shows, the half vector stands within about 20° of the
// normal, so a slope reaches it at a third of its length: there a strong edge tilted toward the
// highlight brightens the glare by about half and one tilted away dims it as much, while paint
// noise moves it under 10 % (a Blinn lobe of exponent 20 would move 20× the slope, all grain).
// The bounds keep the glare where stock puts it, glinting along the edges.
const SPEC_GAIN: f32 = 6.0;
const GLARE_MIN: f32 = 0.5;
const GLARE_MAX: f32 = 1.6;

// The model lanes' sheen: its peak as a share of the sun colour, its Blinn exponent, and the warm
// tint that keeps it from reading as white plastic. The stock models have no specular, so this is
// the look's one addition there, narrow and gated to bright texels.
const MODEL_SHEEN: f32 = 0.3;
const MODEL_SHININESS: f32 = 24.0;
const WARM: vec3<f32> = vec3<f32>(1.0, 0.92, 0.78);
// Terrain's small per-pixel sheen on the relief normal, beside its stock Gouraud glare: row 9's
// colour and exponent at this share, under the same `_s` mask, for a sharp hot spot that glints
// on the relief. More would wash the lit ground to cream, as row 9 per pixel unscaled does.
const TERRAIN_SHEEN: f32 = 0.15;
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
    // Two selects, no `&` of bool vectors: SPIR-V lowers that to a vector OpLogicalAnd, which
    // naga's own SPIR-V reader rejects, and GLSL has no such operator (naga polyfills it).
    let cut = vec4<f32>(l.a, r.a, dn.a, up.a) < vec4<f32>(0.5);
    return select(h, select(h, vec4<f32>(hc), cut), coverage);
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
    // The surface normal as given: unit, or zero (an authored zero normal). The stock lighting
    // reads it unchanged.
    normal: vec3<f32>,
    // The relief's tilt of it, in the surface plane: `normal + slope` is the bumped surface's
    // normal, unnormalised. Linear in the heights, so a mirrored bump carries the opposite slope;
    // zero where `normal` is.
    slope: vec3<f32>,
    // The cavity factor, in [1 − CAVITY_MAX, 1 + CAVITY_MAX].
    cavity: f32,
}

// `n` the surface normal, `t`/`b` the frame of increasing u and v, `h` the tap heights against
// the centre's `hc`. The displaced surface's normal is N − ∇h along the surface, at a slope of
// BUMP per unit of luminance change across one sampled texel.
fn relief(n: vec3<f32>, t: vec3<f32>, b: vec3<f32>, h: vec4<f32>, hc: f32) -> Relief {
    let g = 0.5 * vec2<f32>(h.y - h.x, h.w - h.z);
    let k = select(0.0, BUMP * RELIEF, dot(n, n) > 1e-12);
    let lap = 0.25 * (h.x + h.y + h.z + h.w) - hc;
    let cavity = clamp(1.0 - CAVITY * RELIEF * lap, 1.0 - CAVITY_MAX, 1.0 + CAVITY_MAX);
    return Relief(n, -k * (g.x * t + g.y * b), cavity);
}

// The relief's absence: the normal as given, no slope, no cavity.
fn unrelieved(n: vec3<f32>) -> Relief {
    return Relief(n, vec3<f32>(0.0), 1.0);
}

// The bumped normal, unit (zero where the surface normal is), for the terms that are not linear
// in it: the sheens' Blinn lobes.
fn tilted(r: Relief) -> vec3<f32> {
    let p = r.normal + r.slope;
    let l2 = dot(p, p);
    return select(
        vec3<f32>(0.0),
        p * inverseSqrt(max(l2, 1e-24)),
        l2 > 1e-12 && dot(r.normal, r.normal) > 1e-12,
    );
}

// The relief's change to a Lambert term `max(N·d, 0)` toward the unit direction `d`: the slope's
// own projection, so it averages to zero where the light reaches the bumps, not the drop a
// renormalised tilt and the max() would take. It fades out over the terminator (a face the light
// does not reach shows no relief from it) and never takes more than the face receives.
fn lambert(r: Relief, d: vec3<f32>) -> f32 {
    let nd = dot(r.normal, d);
    return max(RELIEF_GAIN * dot(r.slope, d) * smoothstep(0.0, 0.2, nd), -max(nd, 0.0));
}

// The relief's change to the order-2 SH basis the model lanes light with (the normal, `quad` and
// `x² − y²`, as wow_model.wgsl builds them), scaled as `lambert` is: a lane's lobe dotted with it
// moves by its own derivative along the slope. The lobe is quadratic in the normal and this is
// its linear part, so a bump and its mirror image cancel exactly.
struct ShDelta {
    linear: vec3<f32>,
    quad: vec4<f32>,
    // The `x² − y²` term's (naga's writeback renames an identifier ending in a digit).
    saddle: f32,
}

fn sh_delta(r: Relief) -> ShDelta {
    let n = r.normal;
    let s = RELIEF_GAIN * r.slope;
    return ShDelta(
        s,
        vec4<f32>(
            n.x * s.y + n.y * s.x,
            n.y * s.z + n.z * s.y,
            2.0 * n.z * s.z,
            n.x * s.z + n.z * s.x,
        ),
        2.0 * (n.x * s.x - n.y * s.y),
    );
}

// The stock specular's scale at a pixel, `h` the unit half vector: up where the relief tilts
// toward the highlight, down where it tilts away, bounded, so the stock glare keeps its place and
// its weight and sparkles along the edges.
fn glare(r: Relief, h: vec3<f32>) -> f32 {
    return clamp(1.0 + SPEC_GAIN * SHEEN * dot(r.slope, h), GLARE_MIN, GLARE_MAX);
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

// One committed point light at `p` on the relief `r`: the reference's falloff
// `1/(0.7·d + 0.03·d²)`, diffuse only, as `point_light_sum` evaluates it per vertex, with the
// relief's change to its N·L.
fn point_light(p: vec3<f32>, r: Relief, pos: vec3<f32>, rgb: vec3<f32>) -> vec3<f32> {
    let to_light = pos - p;
    let d = length(to_light);
    let atten = 1.0 / max(0.7 * d + 0.03 * d * d, 1e-4);
    let dir = to_light / max(d, 1e-4);
    let nl = max(dot(r.normal, dir), 0.0) + lambert(r, dir);
    return rgb * (atten * nl);
}

// Schlick's fresnel for water (F0 = 0.02): the share of the sky a surface reflects at this view.
fn fresnel(n: vec3<f32>, v: vec3<f32>) -> f32 {
    let c = 1.0 - clamp(dot(n, v), 0.0, 1.0);
    let c2 = c * c;
    return 0.02 + 0.98 * c2 * c2 * c;
}
