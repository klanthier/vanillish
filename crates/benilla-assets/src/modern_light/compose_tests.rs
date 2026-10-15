//! Vanillish: every world surface shader composed and validated by naga, with and without the
//! modern lighting defs, as Bevy's shader cache composes it (naga_oil), over each def set its
//! pipelines take. No GPU is needed, so this is the one place a typo, a missing import, a type
//! error or a colliding interface location in the modern path fails on a machine without the
//! game: the visual check needs the install.
//!
//! Bevy's own modules (`bevy_pbr::*`, `bevy_render::view`) are stubs here: the declarations these
//! shaders use, with Bevy 0.18.1's names and signatures, bodies reduced to something valid.
//! naga does not check derivative uniformity in a fragment stage
//! (`DISABLE_UNIFORMITY_REQ_FOR_FRAGMENT_STAGE`), so the modern path takes its derivatives before
//! any branch or discard by construction, and samples with explicit gradients after that.

use std::collections::HashMap;

use naga_oil::compose::{
    ComposableModuleDescriptor, Composer, NagaModuleDescriptor, ShaderDefValue, ShaderLanguage,
    ShaderType,
};

use super::{ModernLighting, MODERN_DEF, RELIEF_DEF, SHEEN_DEF};

const TERRAIN: &str = include_str!("../shaders/terrain.wgsl");
const MODEL: &str = include_str!("../shaders/wow_model.wgsl");
const LIQUID: &str = include_str!("../shaders/liquid.wgsl");
const WDL: &str = include_str!("../shaders/wdl.wgsl");
const MODERN: &str = include_str!("../shaders/vanillish_modern.wgsl");
/// `benilla-world`'s retained static pass, which imports the shared module too.
const STATIC_GX: &str = include_str!("../../../benilla-world/src/shaders/static_gx.wgsl");

const VIEW: &str = r"
#define_import_path bevy_render::view

struct View {
    clip_from_world: mat4x4<f32>,
    world_from_view: mat4x4<f32>,
    view_from_world: mat4x4<f32>,
    clip_from_view: mat4x4<f32>,
    world_position: vec3<f32>,
    exposure: f32,
    mip_bias: f32,
    frame_count: u32,
}
";

const GLOBALS: &str = r"
#define_import_path bevy_render::globals

struct Globals {
    time: f32,
    delta_time: f32,
    frame_count: u32,
}
";

const MESH_VIEW_BINDINGS: &str = r"
#define_import_path bevy_pbr::mesh_view_bindings

#import bevy_render::{
    view::View,
    globals::Globals,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(11) var<uniform> globals: Globals;
";

/// Bevy 0.18.1's `forward_io.wgsl`, verbatim but for its comments.
const FORWARD_IO: &str = r"
#define_import_path bevy_pbr::forward_io

struct Vertex {
    @builtin(instance_index) instance_index: u32,
#ifdef VERTEX_POSITIONS
    @location(0) position: vec3<f32>,
#endif
#ifdef VERTEX_NORMALS
    @location(1) normal: vec3<f32>,
#endif
#ifdef VERTEX_UVS_A
    @location(2) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(3) uv_b: vec2<f32>,
#endif
#ifdef VERTEX_TANGENTS
    @location(4) tangent: vec4<f32>,
#endif
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
#ifdef SKINNED
    @location(6) joint_indices: vec4<u32>,
    @location(7) joint_weights: vec4<f32>,
#endif
#ifdef MORPH_TARGETS
    @builtin(vertex_index) index: u32,
#endif
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
#ifdef VERTEX_UVS_A
    @location(2) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(3) uv_b: vec2<f32>,
#endif
#ifdef VERTEX_TANGENTS
    @location(4) world_tangent: vec4<f32>,
#endif
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    @location(6) @interpolate(flat) instance_index: u32,
#endif
#ifdef VISIBILITY_RANGE_DITHER
    @location(7) @interpolate(flat) visibility_range_dither: i32,
#endif
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
}
";

const MESH_FUNCTIONS: &str = r"
#define_import_path bevy_pbr::mesh_functions

struct MeshStub {
    world_from_local: mat4x4<f32>,
    tag: u32,
}

@group(2) @binding(0) var<storage> mesh: array<MeshStub>;

fn get_world_from_local(instance_index: u32) -> mat4x4<f32> {
    return mesh[instance_index].world_from_local;
}

fn mesh_position_local_to_world(world_from_local: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
    return world_from_local * vertex_position;
}

fn mesh_normal_local_to_world(vertex_normal: vec3<f32>, instance_index: u32) -> vec3<f32> {
    let m = mesh[instance_index].world_from_local;
    return normalize(mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz) * vertex_normal);
}

fn get_tag(instance_index: u32) -> u32 {
    return mesh[instance_index].tag;
}
";

const VIEW_TRANSFORMATIONS: &str = r"
#define_import_path bevy_pbr::view_transformations

#import bevy_pbr::mesh_view_bindings as view_bindings

fn position_world_to_clip(world_pos: vec3<f32>) -> vec4<f32> {
    return view_bindings::view.clip_from_world * vec4<f32>(world_pos, 1.0);
}

fn view_z_to_depth_ndc(view_z: f32) -> f32 {
    return -view_bindings::view.clip_from_view[3][2] / view_z;
}
";

const PBR_TYPES: &str = r"
#define_import_path bevy_pbr::pbr_types

struct StandardMaterial {
    base_color: vec4<f32>,
    uv_transform: mat3x3<f32>,
    flags: u32,
    alpha_cutoff: f32,
    deferred_lighting_pass_id: u32,
}

const STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT: u32 = 1u << 0u;
const STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS: u32 = 7u << 29u;
const STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE: u32 = 0u << 29u;
const STANDARD_MATERIAL_FLAGS_ALPHA_MODE_MASK: u32 = 1u << 29u;
const STANDARD_MATERIAL_FLAGS_ALPHA_MODE_ALPHA_TO_COVERAGE: u32 = 6u << 29u;

struct PbrInput {
    material: StandardMaterial,
    world_normal: vec3<f32>,
    N: vec3<f32>,
    V: vec3<f32>,
}
";

const PBR_BINDINGS: &str = r"
#define_import_path bevy_pbr::pbr_bindings

#import bevy_pbr::pbr_types::StandardMaterial

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: StandardMaterial;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_color_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_color_sampler: sampler;
";

const PBR_FRAGMENT: &str = r"
#define_import_path bevy_pbr::pbr_fragment

#import bevy_pbr::{
    pbr_types,
    pbr_bindings,
    forward_io::VertexOutput,
    mesh_view_bindings::view,
}

fn pbr_input_from_standard_material(in: VertexOutput, is_front: bool) -> pbr_types::PbrInput {
    var pbr_input: pbr_types::PbrInput;
    pbr_input.material = pbr_bindings::material;
    pbr_input.world_normal = select(-in.world_normal, in.world_normal, is_front);
    pbr_input.N = normalize(pbr_input.world_normal);
    pbr_input.V = normalize(view.world_position - in.world_position.xyz);
#ifdef VERTEX_COLORS
    pbr_input.material.base_color *= in.color;
#endif
#ifdef VERTEX_UVS_A
    if ((pbr_input.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT) != 0u) {
        pbr_input.material.base_color *= textureSampleBias(
            pbr_bindings::base_color_texture,
            pbr_bindings::base_color_sampler,
            in.uv,
            view.mip_bias,
        );
    }
#endif
    return pbr_input;
}
";

const PBR_FUNCTIONS: &str = r"
#define_import_path bevy_pbr::pbr_functions

#import bevy_pbr::pbr_types

fn alpha_discard(material: pbr_types::StandardMaterial, output_color: vec4<f32>) -> vec4<f32> {
    var color = output_color;
    let alpha_mode = material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
    if alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE {
        color.a = 1.0;
    }
#ifdef MAY_DISCARD
    else if alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_MASK ||
            alpha_mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_ALPHA_TO_COVERAGE {
        if color.a >= material.alpha_cutoff {
            color.a = 1.0;
        } else {
            discard;
        }
    }
#endif
    return color;
}
";

/// A composer holding the stubs and the shared module, as Bevy's shader cache holds every loaded
/// library.
fn composer() -> Composer {
    let mut c = Composer::default();
    for (path, source) in [
        ("view.wgsl", VIEW),
        ("globals.wgsl", GLOBALS),
        ("mesh_view_bindings.wgsl", MESH_VIEW_BINDINGS),
        ("forward_io.wgsl", FORWARD_IO),
        ("mesh_functions.wgsl", MESH_FUNCTIONS),
        ("view_transformations.wgsl", VIEW_TRANSFORMATIONS),
        ("pbr_types.wgsl", PBR_TYPES),
        ("pbr_bindings.wgsl", PBR_BINDINGS),
        ("pbr_fragment.wgsl", PBR_FRAGMENT),
        ("pbr_functions.wgsl", PBR_FUNCTIONS),
        ("vanillish_modern.wgsl", MODERN),
    ] {
        if let Err(e) = c.add_composable_module(ComposableModuleDescriptor {
            source,
            file_path: path,
            language: ShaderLanguage::Wgsl,
            ..Default::default()
        }) {
            panic!("{path}: {}", e.emit_to_string(&c));
        }
    }
    c
}

/// The look's defs as naga_oil takes them: the same values [`ModernLighting::shader_defs`] hands
/// a pipeline, converted the way Bevy converts `ShaderDefVal`.
fn look_defs(look: ModernLighting) -> HashMap<String, ShaderDefValue> {
    look.shader_defs()
        .into_iter()
        .map(|d| match d {
            bevy::shader::ShaderDefVal::Bool(k, v) => (k, ShaderDefValue::Bool(v)),
            bevy::shader::ShaderDefVal::Int(k, v) => (k, ShaderDefValue::Int(v)),
            bevy::shader::ShaderDefVal::UInt(k, v) => (k, ShaderDefValue::UInt(v)),
        })
        .collect()
}

/// Compose `source` under `defs` plus the look's defs, or the error naga_oil reports.
fn try_compose(
    name: &str,
    source: &str,
    defs: &[&str],
    mut shader_defs: HashMap<String, ShaderDefValue>,
) -> Result<naga::Module, String> {
    for d in defs {
        shader_defs.insert((*d).to_string(), ShaderDefValue::Bool(true));
    }
    shader_defs.insert("MATERIAL_BIND_GROUP".into(), ShaderDefValue::UInt(3));
    let mut c = composer();
    c.make_naga_module(NagaModuleDescriptor {
        source,
        file_path: name,
        shader_type: ShaderType::Wgsl,
        shader_defs,
        additional_imports: &[],
    })
    .map_err(|e| e.emit_to_string(&c))
}

/// Compose `source` under `defs` plus the look's, validate it, check that the modern path is in
/// it exactly when the look is on (`modern_path`: the shader has one), and that its interface fits
/// the 16 inter-stage locations WebGPU guarantees.
fn compose(name: &str, source: &str, defs: &[&str], look: ModernLighting, modern_path: bool) {
    let module = match try_compose(name, source, defs, look_defs(look)) {
        Ok(m) => m,
        Err(e) => panic!("{name} with {defs:?}, {look:?} does not compose:\n{e}"),
    };
    // naga_oil decorates an imported function's name as `<stem>X_naga_oil_mod_X<module>X`; these
    // stems are the shared module's alone (liquid uses `fresnel`, the others `derivs`, `luma`).
    let relieved = module.functions.iter().any(|(_, f)| {
        f.name.as_deref().is_some_and(|n| {
            n.split_once("X_naga_oil_mod_X")
                .is_some_and(|(stem, _)| matches!(stem, "fresnel" | "derivs" | "luma"))
        })
    });
    assert_eq!(
        relieved,
        modern_path && look.on,
        "{name} with {defs:?}, {look:?}: the shared module is {} the composed shader",
        if relieved { "in" } else { "missing from" }
    );
    assert_eq!(
        module.entry_points.len(),
        2,
        "{name}: a vertex and a fragment stage"
    );
    for ep in &module.entry_points {
        for arg in &ep.function.arguments {
            let members = match &module.types[arg.ty].inner {
                naga::TypeInner::Struct { members, .. } => members.clone(),
                _ => continue,
            };
            for m in members {
                if let Some(naga::Binding::Location { location, .. }) = m.binding {
                    assert!(
                        location < 16,
                        "{name}: `{:?}` at location {location}, past WebGPU's 16",
                        m.name
                    );
                }
            }
        }
    }
}

/// The look's two settings the tests compose: the tuned defaults, and the strengths' extremes.
fn modern_looks() -> [ModernLighting; 2] {
    [
        ModernLighting {
            on: true,
            relief: 100,
            sheen: 100,
        },
        ModernLighting {
            on: true,
            relief: 0,
            sheen: *super::PERCENT_RANGE.end(),
        },
    ]
}

fn every_look() -> Vec<ModernLighting> {
    let mut v = vec![ModernLighting::STOCK];
    v.extend(modern_looks());
    v
}

#[test]
fn the_shared_module_takes_its_strengths_from_the_defs() {
    for def in [RELIEF_DEF, SHEEN_DEF] {
        assert!(
            MODERN.contains(&format!("#{{{def}}}")),
            "vanillish_modern.wgsl no longer reads {def}"
        );
    }
    for (name, source) in [
        ("terrain", TERRAIN),
        ("wow_model", MODEL),
        ("liquid", LIQUID),
        ("static_gx", STATIC_GX),
    ] {
        assert!(
            source.contains(&format!("#ifdef {MODERN_DEF}"))
                && source.contains("#import benilla_assets::vanillish_modern as vm"),
            "{name}.wgsl lost its modern path"
        );
    }
}

#[test]
fn terrain_composes_with_and_without_the_look() {
    for look in every_look() {
        compose(
            "terrain.wgsl",
            TERRAIN,
            &[
                "VERTEX_POSITIONS",
                "VERTEX_NORMALS",
                "VERTEX_UVS",
                "VERTEX_UVS_A",
                "VERTEX_UVS_B",
                "VERTEX_COLORS",
            ],
            look,
            true,
        );
    }
}

#[test]
fn the_model_lane_composes_with_and_without_the_look_on_every_layout() {
    const BASE: &[&str] = &[
        "VERTEX_POSITIONS",
        "VERTEX_NORMALS",
        "VERTEX_OUTPUT_INSTANCE_INDEX",
        "MAY_DISCARD",
    ];
    const UVS: &[&str] = &["VERTEX_UVS", "VERTEX_UVS_A"];
    let layouts: [&[&str]; 7] = [
        &[],
        UVS,
        &["VERTEX_UVS", "VERTEX_UVS_A", "VERTEX_COLORS"],
        &[
            "VERTEX_UVS",
            "VERTEX_UVS_A",
            "VERTEX_UVS_B",
            "VERTEX_COLORS",
        ],
        &[
            "VERTEX_UVS",
            "VERTEX_UVS_A",
            "WOW_RIG_SKIN",
            "WOW_WATER_CLIP",
        ],
        &[
            "VERTEX_UVS",
            "VERTEX_UVS_A",
            "VERTEX_COLORS",
            "WOW_MERGED_FADE",
            "WOW_MERGED_SLOT",
        ],
        &["VERTEX_UVS", "VERTEX_UVS_A", "WOW_SKY_DEPTH"],
    ];
    for look in every_look() {
        for layout in layouts {
            let defs: Vec<&str> = BASE.iter().chain(layout).copied().collect();
            compose("wow_model.wgsl", MODEL, &defs, look, true);
        }
    }
}

#[test]
fn liquid_composes_with_and_without_the_look() {
    for look in every_look() {
        for colors in [false, true] {
            let mut defs = vec![
                "VERTEX_POSITIONS",
                "VERTEX_NORMALS",
                "VERTEX_UVS",
                "VERTEX_UVS_A",
                "VERTEX_UVS_B",
            ];
            if colors {
                defs.push("VERTEX_COLORS");
            }
            compose("liquid.wgsl", LIQUID, &defs, look, true);
        }
    }
}

#[test]
fn wdl_composes_with_and_without_the_look() {
    for look in every_look() {
        compose(
            "wdl.wgsl",
            WDL,
            &["VERTEX_POSITIONS", "VERTEX_NORMALS"],
            look,
            false,
        );
    }
}

#[test]
fn static_gx_composes_with_and_without_the_look() {
    for look in every_look() {
        for cutout in [false, true] {
            let defs: &[&str] = if cutout { &["GX_CUTOUT"] } else { &[] };
            compose("static_gx.wgsl", STATIC_GX, defs, look, true);
        }
    }
}

/// The modern path really is composed under the def: without the strengths it names, it fails.
#[test]
fn the_modern_path_needs_its_strength_defs() {
    let only_switch = HashMap::from([(MODERN_DEF.to_string(), ShaderDefValue::Bool(true))]);
    let err = try_compose(
        "terrain.wgsl",
        TERRAIN,
        &[
            "VERTEX_POSITIONS",
            "VERTEX_NORMALS",
            "VERTEX_UVS",
            "VERTEX_UVS_A",
            "VERTEX_UVS_B",
            "VERTEX_COLORS",
        ],
        only_switch,
    )
    .expect_err("VANILLISH_MODERN without VANILLISH_RELIEF must not compose");
    assert!(
        err.contains(RELIEF_DEF) || err.contains(SHEEN_DEF),
        "the failure names the missing strength: {err}"
    );
}
