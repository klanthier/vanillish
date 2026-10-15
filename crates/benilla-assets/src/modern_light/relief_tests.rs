//! Vanillish: the modern look's relief maths (`shaders/vanillish_modern.wgsl`) on the host. A Rust
//! twin of each relief term, checked for what the look promises: the stock lighting is the
//! baseline and the relief moves the light around it, never its average, so a surface keeps its
//! stock brightness and glare while its cracks and seams read as 3D. The twin takes its strengths
//! from the WGSL's own constants, so a retune there is a retune here, and the calibration tests
//! below fail when the strengths drift from the reasoned defaults (`a_strong_edge_reads_…`).

const MODERN: &str = include_str!("../shaders/vanillish_modern.wgsl");

type V3 = [f32; 3];

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn normalize(a: V3) -> V3 {
    scale(a, 1.0 / dot(a, a).sqrt())
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A `const NAME: f32 = value;` of the shared module, as the shaders compile it.
fn wgsl_const(name: &str) -> f32 {
    let key = format!("const {name}: f32 = ");
    let line = MODERN
        .lines()
        .find_map(|l| l.trim_start().strip_prefix(key.as_str()))
        .unwrap_or_else(|| panic!("vanillish_modern.wgsl has no `const {name}: f32`"));
    let value = line.trim_end().trim_end_matches(';').trim();
    value
        .parse()
        .unwrap_or_else(|e| panic!("{name} = {value:?}: {e}"))
}

/// The relief's strengths, read from the WGSL, at the session's two percents.
struct Look {
    bump: f32,
    relief_gain: f32,
    cavity: f32,
    cavity_max: f32,
    spec_gain: f32,
    glare_min: f32,
    glare_max: f32,
    /// `VANILLISH_RELIEF` / 100.
    relief: f32,
    /// `VANILLISH_SHEEN` / 100.
    sheen: f32,
}

impl Look {
    fn at(relief_percent: u32, sheen_percent: u32) -> Self {
        Self {
            bump: wgsl_const("BUMP"),
            relief_gain: wgsl_const("RELIEF_GAIN"),
            cavity: wgsl_const("CAVITY"),
            cavity_max: wgsl_const("CAVITY_MAX"),
            spec_gain: wgsl_const("SPEC_GAIN"),
            glare_min: wgsl_const("GLARE_MIN"),
            glare_max: wgsl_const("GLARE_MAX"),
            relief: relief_percent as f32 / 100.0,
            sheen: sheen_percent as f32 / 100.0,
        }
    }

    fn tuned() -> Self {
        Self::at(100, 100)
    }
}

/// `vm::Relief`.
#[derive(Clone, Copy, Debug)]
struct Relief {
    normal: V3,
    slope: V3,
    cavity: f32,
}

/// `vm::relief`: `h` the taps (left, right, down, up), `hc` the centre.
fn relief(k: &Look, n: V3, t: V3, b: V3, h: [f32; 4], hc: f32) -> Relief {
    let g = [0.5 * (h[1] - h[0]), 0.5 * (h[3] - h[2])];
    let tilt = if dot(n, n) > 1e-12 {
        k.bump * k.relief
    } else {
        0.0
    };
    let lap = 0.25 * (h[0] + h[1] + h[2] + h[3]) - hc;
    let cavity = (1.0 - k.cavity * k.relief * lap).clamp(1.0 - k.cavity_max, 1.0 + k.cavity_max);
    Relief {
        normal: n,
        slope: scale(add(scale(t, g[0]), scale(b, g[1])), -tilt),
        cavity,
    }
}

/// `vm::lambert`.
fn lambert(k: &Look, r: Relief, d: V3) -> f32 {
    let nd = dot(r.normal, d);
    (k.relief_gain * dot(r.slope, d) * smoothstep(0.0, 0.2, nd)).max(-nd.max(0.0))
}

/// `vm::glare`.
fn glare(k: &Look, r: Relief, h: V3) -> f32 {
    (1.0 + k.spec_gain * k.sheen * dot(r.slope, h)).clamp(k.glare_min, k.glare_max)
}

/// `vm::sh_delta`: the change to (the normal, `quad`, `x² − y²`).
fn sh_delta(k: &Look, r: Relief) -> (V3, [f32; 4], f32) {
    let n = r.normal;
    let s = scale(r.slope, k.relief_gain);
    (
        s,
        [
            n[0] * s[1] + n[1] * s[0],
            n[1] * s[2] + n[2] * s[1],
            2.0 * n[2] * s[2],
            n[0] * s[2] + n[2] * s[0],
        ],
        2.0 * (n[0] * s[0] - n[1] * s[1]),
    )
}

/// Flat ground, as terrain.wgsl frames it: Bevy's +Y up, u along +X, v along +Z.
const UP: V3 = [0.0, 1.0, 0.0];
const U: V3 = [1.0, 0.0, 0.0];
const V: V3 = [0.0, 0.0, 1.0];

/// Toward a sun `elevation` degrees up, from the azimuth `azimuth` degrees off +X toward +Z.
fn sun(elevation: f32, azimuth: f32) -> V3 {
    let (e, a) = (elevation.to_radians(), azimuth.to_radians());
    [e.cos() * a.cos(), e.sin(), e.cos() * a.sin()]
}

/// The relief at texel (x, y) of a periodic height field, its four taps one texel out.
fn relief_at(k: &Look, field: &[Vec<f32>], x: usize, y: usize) -> Relief {
    let (w, hgt) = (field[0].len(), field.len());
    let at = |x: usize, y: usize| field[y % hgt][x % w];
    let h = [
        at(x + w - 1, y),
        at(x + 1, y),
        at(x, y + hgt - 1),
        at(x, y + 1),
    ];
    relief(k, UP, U, V, h, at(x, y))
}

/// A periodic 64² field of painted ground: a floor, a raised boulder, a crack across it, and
/// a little deterministic paint noise.
fn ground() -> Vec<Vec<f32>> {
    let n = 64;
    (0..n)
        .map(|y| {
            (0..n)
                .map(|x| {
                    let (fx, fy) = (x as f32, y as f32);
                    let r2 = (fx - 20.0).powi(2) + (fy - 30.0).powi(2);
                    let boulder = 0.3 * (-r2 / 40.0).exp();
                    let crack = if x == 45 { -0.25 } else { 0.0 };
                    let noise = 0.02 * ((x * 7 + y * 13) % 5) as f32 - 0.04;
                    0.45 + boulder + crack + noise
                })
                .collect()
        })
        .collect()
}

#[test]
fn the_relief_moves_the_sun_but_not_its_mean() {
    let k = Look::tuned();
    let field = ground();
    for (elevation, azimuth) in [(30.0, 0.0), (20.0, 135.0), (37.0, 250.0)] {
        let l = sun(elevation, azimuth);
        let (mut sum, mut lo, mut hi, mut count) = (0.0f64, 0.0f32, 0.0f32, 0.0f64);
        for y in 0..field.len() {
            for x in 0..field[0].len() {
                let d = lambert(&k, relief_at(&k, &field, x, y), l);
                sum += d as f64;
                lo = lo.min(d);
                hi = hi.max(d);
                count += 1.0;
            }
        }
        let mean = sum / count;
        // A slope is a central difference, so over a whole (periodic) texture the slopes sum to
        // zero, and so does the light they move.
        assert!(
            mean.abs() < 1e-6,
            "sun {elevation}° at {azimuth}°: the relief moves the mean N·L by {mean}"
        );
        assert!(
            lo < -0.1 && hi > 0.1,
            "sun {elevation}° at {azimuth}°: the relief is visible ({lo}..{hi})"
        );
    }
}

/// The look's first try tilted the normal and lit it, `max(normalize(N + slope)·L, 0)`: the
/// renormalisation shortens every tilted normal's reach toward the sun, so it darkened the ground
/// it bumped. The zero-mean term exists because of this.
#[test]
fn a_renormalised_tilt_would_darken_the_ground() {
    let k = Look::tuned();
    let field = ground();
    let l = sun(30.0, 0.0);
    let mut drop = 0.0f64;
    for y in 0..field.len() {
        for x in 0..field[0].len() {
            let r = relief_at(&k, &field, x, y);
            let tilted = normalize(add(r.normal, r.slope));
            drop += (dot(tilted, l).max(0.0) - dot(r.normal, l).max(0.0)) as f64;
        }
    }
    assert!(drop < 0.0, "a renormalised tilt darkens on average");
}

#[test]
fn a_symmetric_bump_cancels_texel_for_texel() {
    let k = Look::tuned();
    let l = sun(30.0, 20.0);
    // A bump and a dent, each mirror-symmetric: the sunward side's gain is the far side's loss.
    for amp in [0.3f32, -0.3] {
        let row: Vec<f32> = (0..32)
            .map(|x| 0.4 + amp * (-((x as f32 - 16.0).powi(2)) / 6.0).exp())
            .collect();
        let field = vec![row; 3];
        for i in 1..16 {
            let a = lambert(&k, relief_at(&k, &field, 16 - i, 1), l);
            let b = lambert(&k, relief_at(&k, &field, 16 + i, 1), l);
            assert!(
                (a + b).abs() < 1e-6,
                "amp {amp}, ±{i} texels: {a} and {b} do not cancel"
            );
        }
        let peak = relief_at(&k, &field, 16, 1);
        assert_eq!(lambert(&k, peak, l), 0.0, "the top of the bump is level");
    }
}

/// The calibration: with BUMP and RELIEF_GAIN as tuned, a strong texel edge reads at play
/// distance and paint noise does not grain. A typical lit diffuse: ambient 0.4, sun 0.7 at 30°.
#[test]
fn a_strong_edge_reads_and_paint_noise_does_not() {
    let k = Look::tuned();
    let (ambient, diffuse) = (0.4f32, 0.7f32);
    let l = sun(30.0, 0.0);
    let lit = ambient + diffuse * dot(UP, l);
    // The change to the lit diffuse at a texel beside a step of `step` running across u (the
    // edge's line turned `angle` degrees from facing the sun).
    let change = |step: f32, angle: f32| {
        let a = angle.to_radians();
        let t = [a.cos(), 0.0, a.sin()];
        let b = [-a.sin(), 0.0, a.cos()];
        let r = relief(&k, UP, t, b, [0.5, 0.5 - step, 0.5, 0.5], 0.5);
        diffuse * lambert(&k, r, l) / lit
    };
    let facing = change(0.25, 0.0);
    let diagonal = change(0.25, 45.0);
    let noise = change(0.04, 0.0);
    assert!(
        (0.25..=0.35).contains(&facing),
        "a strong edge facing the sun moves the lit diffuse by {:.1} %",
        100.0 * facing
    );
    assert!(
        (0.15..=0.25).contains(&diagonal),
        "a strong edge diagonal to the sun moves the lit diffuse by {:.1} %",
        100.0 * diagonal
    );
    assert!(
        noise.abs() < 0.05,
        "paint noise moves the lit diffuse by {:.1} %",
        100.0 * noise
    );
    // Facing the other way, the same edge darkens by as much.
    assert!((change(-0.25, 0.0) + facing).abs() < 1e-6);
}

#[test]
fn the_cavity_darkens_cracks_lightens_ridges_and_averages_to_one() {
    let k = Look::tuned();
    // A gentle field (no factor reaches the cap): the Laplacian sums to zero over a whole texture,
    // so the factors average to one.
    let field: Vec<Vec<f32>> = (0..32)
        .map(|y| {
            (0..32)
                .map(|x| 0.5 + 0.04 * ((x * 5 + y * 11) % 7) as f32 / 7.0)
                .collect()
        })
        .collect();
    let mut sum = 0.0f64;
    for y in 0..32 {
        for x in 0..32 {
            let c = relief_at(&k, &field, x, y).cavity;
            assert!((c - 1.0).abs() < k.cavity_max, "{c} reaches the cap");
            sum += c as f64;
        }
    }
    assert!(
        (sum / 1024.0 - 1.0).abs() < 1e-6,
        "mean cavity {}",
        sum / 1024.0
    );
    // A crack 0.1 under its neighbours is darker, a ridge as far over lighter, by as much.
    let crack = relief(&k, UP, U, V, [0.5; 4], 0.4).cavity;
    let ridge = relief(&k, UP, U, V, [0.5; 4], 0.6).cavity;
    assert!(crack < 1.0 && ridge > 1.0);
    assert!((crack + ridge - 2.0).abs() < 1e-6);
    // Capped both ways.
    assert_eq!(
        relief(&k, UP, U, V, [0.9; 4], 0.0).cavity,
        1.0 - k.cavity_max
    );
    assert_eq!(
        relief(&k, UP, U, V, [0.0; 4], 0.9).cavity,
        1.0 + k.cavity_max
    );
}

#[test]
fn the_glare_keeps_the_stock_specular_and_sparkles_on_edges() {
    let k = Look::tuned();
    // A half vector where row 9's glare shows on level ground: N·H 0.95.
    let h = normalize([0.33, 1.0, 0.0]);
    // Level ground: the stock glare, unscaled.
    let flat = relief(&k, UP, U, V, [0.5; 4], 0.5);
    assert_eq!(glare(&k, flat, h), 1.0);
    // A gentle slope and its mirror image average to the stock glare.
    let up = relief(&k, UP, U, V, [0.5, 0.52, 0.5, 0.5], 0.51);
    let down = relief(&k, UP, U, V, [0.52, 0.5, 0.5, 0.5], 0.51);
    let (a, b) = (glare(&k, up, h), glare(&k, down, h));
    assert!(a != 1.0 && ((a + b) * 0.5 - 1.0).abs() < 1e-6, "{a}, {b}");
    // A strong edge glints at the top bound, or dims to the bottom one.
    let toward = relief(&k, UP, U, V, [0.8, 0.2, 0.5, 0.5], 0.5);
    let away = relief(&k, UP, U, V, [0.2, 0.8, 0.5, 0.5], 0.5);
    assert_eq!(glare(&k, toward, h), k.glare_max);
    assert_eq!(glare(&k, away, h), k.glare_min);
    // The sheen lever at 0 leaves the stock glare alone.
    assert_eq!(glare(&Look::at(100, 0), toward, h), 1.0);
    // The calibration: a strong edge (a 0.25 step) moves the glare by a third or more, paint
    // noise (0.04 steps) by under a tenth.
    let edge = glare(&k, relief(&k, UP, U, V, [0.5, 0.25, 0.5, 0.5], 0.4), h) - 1.0;
    let noise = glare(&k, relief(&k, UP, U, V, [0.5, 0.46, 0.5, 0.5], 0.5), h) - 1.0;
    assert!(
        edge >= 0.33,
        "a strong edge moves the glare by {:.0} %",
        100.0 * edge
    );
    assert!(
        noise.abs() < 0.1,
        "paint noise moves the glare by {:.0} %",
        100.0 * noise
    );
}

/// The model lanes' SH lobes: the change `sh_delta` carries is the lobe's own linear part, the
/// half difference of the lobe at the two mirrored tilts, so they cancel exactly.
#[test]
fn the_sh_change_is_the_lobes_own_linear_part() {
    let k = Look::tuned();
    // An arbitrary order-2 lobe: linear band, the four quadratic terms, x² − y².
    let (lin, quad, saddle) = ([0.31, 0.52, -0.17], [0.11, -0.23, 0.07, 0.19], -0.13);
    let lobe = |n: V3| {
        let q = [n[0] * n[1], n[1] * n[2], n[2] * n[2], n[0] * n[2]];
        dot(lin, n)
            + quad[0] * q[0]
            + quad[1] * q[1]
            + quad[2] * q[2]
            + quad[3] * q[3]
            + saddle * (n[0] * n[0] - n[1] * n[1])
    };
    let n = normalize([0.3, 0.8, -0.5]);
    let t = normalize([0.8, -0.3, 0.0]);
    let b = [
        n[1] * t[2] - n[2] * t[1],
        n[2] * t[0] - n[0] * t[2],
        n[0] * t[1] - n[1] * t[0],
    ];
    let r = relief(&k, n, t, b, [0.3, 0.6, 0.55, 0.4], 0.5);
    let (dn, dq, ds) = sh_delta(&k, r);
    let change = dot(lin, dn)
        + quad[0] * dq[0]
        + quad[1] * dq[1]
        + quad[2] * dq[2]
        + quad[3] * dq[3]
        + saddle * ds;
    let s = scale(r.slope, k.relief_gain);
    let half_difference = 0.5 * (lobe(add(n, s)) - lobe(add(n, scale(s, -1.0))));
    assert!(change.abs() > 0.01, "the relief moves the lobe ({change})");
    assert!(
        (change - half_difference).abs() < 1e-5,
        "{change} is not the lobe's linear part {half_difference}"
    );
}

#[test]
fn the_light_never_shows_relief_where_it_does_not_reach() {
    let k = Look::tuned();
    let steep = relief(&k, UP, U, V, [0.9, 0.1, 0.9, 0.1], 0.5);
    // Behind the terminator: nothing to move.
    for elevation in [-30.0, -5.0, 0.0] {
        assert_eq!(lambert(&k, steep, sun(elevation, 0.0)), 0.0);
        assert_eq!(lambert(&k, steep, sun(elevation, 180.0)), 0.0);
    }
    // In front of it, never below no light at all: what a point light sums is never negative.
    for elevation in [2.0, 5.0, 10.0, 20.0, 45.0, 80.0] {
        for azimuth in [0.0, 90.0, 180.0, 270.0] {
            let l = sun(elevation, azimuth);
            assert!(dot(UP, l).max(0.0) + lambert(&k, steep, l) >= 0.0);
        }
    }
}

#[test]
fn the_relief_lever_at_zero_is_the_stock_look() {
    let k = Look::at(0, 100);
    let r = relief(&k, UP, U, V, [0.9, 0.1, 0.2, 0.7], 0.3);
    assert_eq!(r.slope, [0.0; 3]);
    assert_eq!(r.cavity, 1.0);
    assert_eq!(lambert(&k, r, sun(30.0, 0.0)), 0.0);
    assert_eq!(glare(&k, r, UP), 1.0);
    // An authored zero normal takes no slope either.
    let zero = relief(&Look::tuned(), [0.0; 3], U, V, [0.9, 0.1, 0.2, 0.7], 0.3);
    assert_eq!(zero.slope, [0.0; 3]);
}
