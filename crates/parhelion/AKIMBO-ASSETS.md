# Akimbo asset compiler

The compiler builds private paired geometry, materials and vertex programs from the
selected installation's native assets. It also derives the native runtime's muzzle,
ejection, tracer and firing-effect bindings from the final weapon graph. No generated
game shaders, decoded models or private packages are shipped in this repository.

This implements same-weapon pairs through the original independent SMG adapter,
the generalized direct-gun adapter and the finite-burst adapter. Compatibility is determined by the resolved
geometry, shader, rig, barrel and effect structures. There is no supported-weapon-name
list; a weapon family label does not establish compatibility. See
[Akimbo authoring](AKIMBO-AUTHORING.md) for recipe and runtime requirements.

The source transforms and offline validation described below are implemented.
Gameplay acceptance of newly generated packages remains a separate requirement;
the shader and parser results below are not an in-game test result.

## Implemented pipeline

### Final runtime and appearance

`item/emission/reskin/akimbo_profile.rs` assesses the compiled runtime entity and
selected content group. Reads prefer pending private allocations over source
packages, so copied or edited components cannot pass by inspecting stale donor
data. It checks the independent adapter's native component ownership, barrel
layout, projectile route, content selection and audited first-person rig.

The generalized gun adapter also proves the authored firing tier. All four native
bullet inputs must copy the same validated translator column. Constant zero and
one follow the native single-round convention. For a variable RPM column, the
compiler resolves the RPM hash through the installed typed stat-definition table
and samples the final emitted item's stat value with the translator's interpolation
rule. Fractional results, conditional stat programs, pins affecting the selected
input and unknown routing are rejected. Finite integral counts of 2–64 use a
separate adapter requiring matching native count routes, compatible selected
content, digital primary input and ordinary magazine reloads. The runtime
separately checks all four live inputs before issuing a shot, including later
perk/stat transitions.

`item/emission/reskin/akimbo_core.rs` gives finite-burst controllers a private
startup identity before their presentation model is available. It clones the
native core with the existing typed owner-reference retargeter, preserves every
non-owner byte and enrolls its private entity as a runtime dependency. Deployment
requires both emitted ownership and a generated-entry domain; an overlay package
alone cannot establish that a core entry is private.

The initial adapter uses the weapon's own native appearance and presentation.
Borrowed rigs, borrowed appearances, imported models, additional behaviors, moved
markers and held offsets require additional compatibility work. The common shader
glow edit is composed with the paired material when requested.

`item/emission/reskin.rs` traverses the resolved art assignments and allocates
private relations, entities, model owners and models through Parhelion's existing
linker. It preserves art alternatives and native loading companions. A generated
native registration uses an actual allocated model-owner identity, not a fixed
historical private tag. Every final appearance entity is supplied to visual
assessment, including marker-only parts.

### Geometry and quantization

`weapon/akimbo/geometry.rs` accepts native rigid eight-byte position rows and packed
twelve-byte rows with two bone influences. Rigid rows contain three signed 16-bit
coordinates and one bone lane. Packed rows preserve the native trailing bytes
`[bone0, bone1, weight0, weight1]`; weights must sum to 255. Both bone indices are
checked even when their weight is zero. Indices must be below 64, fit the descriptor
palette and leave the protocol marker unused. The emitter requires native 16-bit
index buffers; other weighted formats and auxiliary streams remain rejected.

`item/emission/reskin/akimbo_layout.rs` proves the packed stream's actual typed GPU
declaration: POSITION0 signed-normalized int16x4 followed by BLENDINDICES0 uint8x4.
It validates every draw's stage layout and attribute stride, rejects instancing and
extra weight streams, and pins the source declaration tables. The packed source's
unused POSITION.w must be 32767. Its private copy replaces that lane with zero or
`0xC0`, leaving both influences and all weight bytes independent of copy flags.

The emitter pairs supported weapon meshes in the selected appearance. It copies
attributes without modification, translates the left copy along local Y, and
encodes that copy with flags `0xC0`. A common scale is computed for all meshes in
each model. Quantization is calculated relative to the original model offset and
checked against a 0.1 mm error bound. UV fields remain unchanged; the scalar and
XYZ position scales, sphere center and enclosing radius are updated together.

The recipe's integer `gap_mm` selects 50–800 mm, defaulting to 360 mm for existing
recipes. The emitter converts it to meters and uses the same value for mesh
duplication, material shaders, shared-effect branches and the schema 5 native
profile. Baked roots remain at local Y=0 and Y=gap. The native pose supplies the
common −gap/2 shift and cancels the original root's camera-space lateral offset;
centering the vertices again would apply the shift twice and detach pose markers.
Vertical and depth animation are preserved. The native left marker excludes the
baked translation, while muzzle/ejection/tracer roots include it exactly once.

Triangle-list winding and triangle-strip restart boundaries are preserved.
Repeated draw ranges are transformed once. Draw order, material identities, stage
and LOD metadata remain attached to the same records; only index ranges and
primitive counts change. Record identities do not grow in this path, so it does
not require the historical merged-record mask expansion. Vertex, index, buffer
and whole-model allocation limits are checked before output is allocated.

The native central reticle is retained once. `shader/reticle.rs` recognizes its
immediate constant buffers, POSITION.xyz quantization and screen-projection
dataflow. Its material and shader remain unchanged, while its positions use the
model's common requantization. Mixed reticle/weapon material meshes retain the
reticle's draw range once while duplicating only the weapon draw ranges. Shared
ranges must agree on their duplication policy; every later range is relocated
when earlier index data grows.
Other unsupported shader shapes are rejected; there is no root-pinning fallback.

### Stock-to-private vertex programs

`weapon/akimbo/shader.rs` and `shader/single_marker.rs` generate the paired skinning
logic from source. `dxbc.rs` now parses and emits SM5 vertex programs while retaining
the existing pixel-program interface.

The transform recognizes a unique native POSITION0 bone decode, its affine-row
indices and their dynamic `cb11` consumers. It validates instruction structure,
register lifetimes, constant-buffer capacity, input signatures and control-flow
scope. It allocates new temporary registers and rewrites only the proven bone-row
reads. Original UV, normal, lighting and projection instructions remain sourced
from the selected program. Unsupported or ambiguous layouts fail with a diagnostic.
The emitter proves the source upload against its model and rig and checks every
original bone lane actually indexed by each material's draws. Materials may use
fewer bones than the whole model. Some stock shaders declare fewer rows than their
bound buffer contains; the declaration alone cannot prove the source upload size.
The explicit source-palette path grows the private declaration to cover the complete
final rig. The shader-only entry point retains its stricter declaration check.
Bounded visibility `if`/`else` branches retain their original zero-output path; palette
consumers must share the decoder's validated branch and live row indices.
Native MIN/MAX clamping, reciprocal square root and fractional animation arithmetic
are retained with operand and palette checks. Numerical tests include negative
fractional values, and the source Whisper program's animated offsets pass a D3D11
comparison across its complete source bone palette after private renumbering.

`shader/two_bone.rs` handles the packed path. It proves the two typed input lanes,
the complete live affine-row addresses for both influences, and that native
POSITION.w is unused. Each influence receives the independently transformed rows;
the original weight conversion, interpolation, normals and projection remain in
the native instruction sequence. Private rig insertion renumbers both source bone
indices. The implementation does not collapse a weighted vertex onto its first bone.

The first protocol uses weapon bone 3, the recipe's selected gap, and a marker scale of
`1.03515625 + mask / 512`. The two mask bits select which hand's moving parts are
active. The generated left transform composes the marker root, inverse native
root and the selected moving bone, with the geometry translation removed through
the matching local pivot. Marker identity, geometry flags, shader separation and
the native pose adapter must agree.

`visual/rig.rs` also checks the weapon-local five-bone skeleton and controls. Bone
roles, hierarchy, bind/inverse-bind agreement, marker calibration, control maps
and defaults must match the native adapter. The marker is a leaf with no attached
markers or named controls. Static model palettes may be expanded to include this
existing skeleton bone only after validating source lanes against the original
palette; the shader must already declare enough constant-buffer capacity.

The generalized adapter derives a dedicated marker from the selected weapon's
equipment and first-person rigs in `visual/pose.rs`. It compares all 72 shared
arm bone names, parents, bind and inverse-bind bytes with the installed reference
rig before retaining the native arm mapping. Equipment/first-person weapon names,
parents and FK/procedural ranges must agree. The weapon's actual bone count,
control count and typed animation snapshot size become descriptor fields.

`parhelion_import::tiger::animation::rig` extends the two native skeletons and
controls together, including instance pose storage, range schemas and per-bone
maps. It preserves the native four-range limit. Inserting the new marker before
the trailing procedural range places it after all native FK weapon bones; audited
source rigs have between one and twelve such bones. Existing FK bones retain
their indices and procedural bones move one index later. Clip control slots stay unchanged; control
outputs, parent references, named/procedural targets, geometry bone lanes and
attachment rows follow the checked bone renumbering. The marker has an identity
equipment bind, a separate default control and no source geometry weights.

`item/emission/reskin/akimbo_pose.rs` links private equipment, arms, animation,
marker and attachment owners and changes the authored pattern's runtime entity.
Typed interior references follow skeleton relocation. Component owner slots and
explicit asset links are patched by structure; equal opaque words are preserved.
After rebasing, every incoming reference to the extended skeleton must name its
private owner, including interior interfaces without a type header at their target.
Leaving a stock owner with a moved offset resolves unrelated stock bytes as a
runtime handle. The linker applies the existing importer proof only to these
already-validated rig references, and retargets nested FK instance/definition
headers through the component-owner serializer. The stock pulse-rifle regression
checks the interior alias, all reciprocal FK headers and unchanged non-owner bytes.
Assigned runtime resources and their native prerequisite graphs are enrolled in
the ordinary runtime dependency root; appearance resources keep their own scope.
The final descriptor names these allocated arms resources. The shared inspect-scene
compiler also extends the scene's equipment skeleton, controls and markers;
failure to create a compatible scene refuses the build instead of emitting a
model palette larger than the scene skeleton.

The generalized native adapter consumes an FK weapon section ending at the
descriptor's marker index, followed by the shifted nonempty procedural range.
The first-person arm ranges remain FK 0–24 and procedural 25–71; weapon root 72
stays fixed. Source-level extension and compiler tests
do not establish independent animation clocks or gameplay acceptance for every
trigger, reload or projectile system.

`item/emission/reskin/akimbo.rs` clones each affected material, shader allocation
header and bytecode entry. Header/payload references remain reciprocal. Optional
glow pixel-program changes are made on this same private material. Texture and dye
bindings stay native; this implementation does not merge texture plates or provide
the historical mixed-donor texture conversion.

### Shared cloak and object effects

`shader/effects.rs` and `shader/two_bone.rs` transform the six native rigid and six
packed-two-influence object-effect vertex programs. Each selector matches all four exact descriptor words from
`cb11[5]`: position offset XYZ and quantization scale. A deterministic decision tree
uses unsigned comparisons to select a bucket of at most eight candidates, then
proves all four words with integer equality. Small bounded leaves limit D3D shader
compilation cost as well as execution work. Tree depth is bounded at 48 comparisons. Only an exact
match enters a second balanced tree selecting the transform contract; no private
marker rows are read before that match. Selectors with equal marker, palette and
separation bits share one transform body. The weighted path performs the lookup
once and reuses its result for both influences.

The compiler accepts up to 4,096 unique generated model selectors and applies an
additional 2 MiB serialized program limit. The original native bone decode and
rows are preserved for unmatched descriptors, including bone lanes above 63.

Before emitting the global shader overlay, the compiler compares each generated
selector against **every source model descriptor** and other authored models. A
collision with unrelated geometry is rejected. Identical generated selectors can
share a branch only when their marker, palette and separation contracts agree.
This census is necessary: the flag bits alone are not a safe global identity test.

### Generated visual bindings

`weapon/akimbo/visual.rs` follows typed arrays rather than searching for bytes that
resemble asset references. It derives:

- Muzzle and ejection frames from the final appearance's marker components,
  falling back to the weapon's native marker set. Conflicting overrides and use of
  the pose marker bone are rejected. Non-root attachments retain their validated
  source bone and bind frame; the native companion composes the corresponding live
  bone relative to each hand and uses the same moving-part mask as the vertex path.
  Additional named local groups preserve native marker order and bounded capacity.
  An absent ejection frame is allowed only when no selector consumes it.
  Native marker rows keep availability at byte 0 separate from the bone index at
  byte 4; rig extension renumbers only the latter. Multiple muzzle rows can select
  the first native row only after proving equal availability and the supported
  root binding. Sorted marker names preserve the engine's binary-search contract.
- The tracer movement component from the selected content's projectile override,
  or the agreed native barrel projectile graph when no override is present. The
  unique movement component must have the runtime adapter's supported layout.
- The selected content's firing sequence, including its typed fixed-slot override
  route, local attachment bindings, particle selector offsets, programs and sizes.
  Muzzle/ejection groups use one marker; additional groups contain at most six
  source frames. Local selectors retain one frame input and at most one endpoint
  input. The program's attachment mapping and group count are checked.

The visual adapter handles native particle, sound, camera and validated control
events. Unrecognized override layouts, event types, attachment mappings and
alternative projectile component layouts fail explicitly.
The runtime receives the generated donor-specific frames and selector table; it
does not substitute IKELOS's particle identities for another donor's effects.

`item/emission/reskin/akimbo_fx.rs` privately clones firing graphs whose source
identities are reserved by historical native adapters. Typed graph references
retain their loading ownership: entity/content roots are runtime dependencies,
while wrapper and sequence assets keep their scoped native companions. The final
visual pass derives the cloned identities again before publishing the profile.

### Registration, dependencies and installation

`weapon/akimbo/deployment.rs` defines the schema 5 runtime descriptor. Each profile
includes the allocated model owner, adapter, gap in millimeters, reload policy,
rig, barrel and visual bindings. Reload policy 1 retains empty-only selection;
policy 2 selects all eligible non-full magazines independently on R; policy 3
selects the lower magazine, or both when both are empty. The recipe's
explicit choice reaches the native profile; changing the default does not reinterpret
old saved `empty_hands` or `non_full_hands` recipes. Strict schema 1–4 readers exist only for
retirement with exact-byte backup and recovery, never for new staging or activation.
Frame values, counts, offsets and identities are bounded; conflicting profiles
that claim one shared firing sequence are rejected. The build records its source
dependencies and emitted packages for compatibility and integrity verification.

The companion capability and supported executable/content build are required in
addition to successful asset compilation. Geometry alone cannot implement per-hand
input, magazines, reload, recoil, hands or firing effects. The ordinary install
planner remains responsible for package composition and deployment; this compiler
does not load a DLL into a running game.

## Verification completed during implementation

Public tests use synthetic geometry, shader and visual fixtures. They exercise
quantization, draw topology, occupied markers, native high bone lanes, invalid
layouts, bounded shader parsing, temporary allocation, effect-selector isolation,
malformed graph arrays, ambiguous components and unsupported particle mappings.
Gap regressions cover 50, 360 and 800 mm, both copies of quantized geometry,
executed main/shared-effect shader branches with a rotated moving bone, unmatched
native effects, saved intent and recipe fingerprint changes. Native tests cover
camera-space lateral centering; live gameplay acceptance is still pending.

Optional local validation used stock package data and historical private outputs
outside the repository:

| Check | Result |
| --- | --- |
| Three supported stock main vertex programs transformed from source | Exact byte-for-byte equality with the corresponding historical IKELOS programs, including DXBC checksums |
| D3D11 WARP creation and main-program execution | All programs accepted; 4,320 vertex cases checked |
| Six shared object-effect programs on unrelated native vertices | 32,760 cases bit-identical, including high bone lanes, near-miss descriptor selectors and disabled branches |
| Paired object-effect positions against the generated main program | 2,880 world positions bit-identical |
| IKELOS visual graph parsing | Exact equality with all 12 native selector tuples, muzzle/ejection frame bits and tracer identity/definition |
| Three other source SMG firing graphs | Distinct tables of 10, 18 and 15 supported local selectors derived without donor-ID gates |
| All 24 distinct source gun equipment/arm rig pairs | Dedicated marker, native instance/control extension, range correspondence and descriptor bounds validated, including bow and machine-gun shapes |
| Sturm declaration-only expansion on D3D11 WARP | 256 vertices across all eight source bones byte-identical; perturbing each frame proves the native program reads the bound buffer beyond its nominal declaration |
| Generalized Sturm and Thunderlord shaders with private bone remapping | 736 vertices across all 8 and 15 source bones byte-identical to stock with a neutral marker |
| Three packed two-influence stock programs | 432 neutral-marker vertices byte-identical to native; 1,152 paired positions agree with independent rigid-influence blending within 0.000002 model units |
| Six packed two-influence object-effect programs | 2,160 unmatched native vertices byte-identical; 8,640 paired positions agree with the generated primary program |
| Balanced shared effects with bounded eight-pin leaves, 257 and 4,096 exact selectors, and four rig contracts | Rigid and weighted programs accepted by WARP; 6,552 matched and 1,280 unmatched vertices byte-identical to the corresponding single-selector or native paths |
| Balanced dispatch over an emitted 24-weapon package | All 219 exact model pins and 11 source-derived contracts preserved; 1,314 matched and 4,380 one-bit near-miss vertices byte-identical to the previous transform or native program |
| Balanced dispatch work bound | A 4,096-selector, 44-contract synthetic layout takes nine tree comparisons and at most eight exact-pin comparisons; unmatched execution reads no private marker rows and adds at most 49 arithmetic instructions to the native fixture |

WARP is Microsoft's software D3D implementation. These results establish compiler
and shader behavior for the exercised inputs; they do not establish hardware GPU
compatibility, clean-package equip behavior, correct live animation, per-hand
ammunition/reload behavior or the native behavior of every source perk.

Before calling a generated profile gameplay-qualified, rebuild it from the chosen
clean installation, read back the emitted dependencies, install with the matching
companion, equip it, fire and reload each hand and both together, switch weapons,
respawn, sprint, and exercise muzzle/tracer placement and cloak effects. Check an
unrelated native weapon in the same session. Newly generated pair inventory/HUD
artwork, mixed donors and other pose protocols require their own compatibility work.
Stock swords select no first-person arms entity and therefore need a separate melee
and third-person presentation path rather than the gun protocol's shared 72-arm prefix.
