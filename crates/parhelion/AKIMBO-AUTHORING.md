# Akimbo authoring

This branch extends Sundial v0.6 (`3f180dfb`) with akimbo recipe intent, checked asset compilation, a Gameplay control, and transactional installation of the generated native bindings. It requires the accompanying Dawn akimbo runtime; the official Dawn v1.7.2 and Sundial v0.6 releases do not provide this capability.

The implementation generalizes the independent-hand route from Dawn's `ikelos-akimbo` work (`87f70dc`). Compatibility is derived from the resolved component, rig, model, shader and effect layouts. There is no donor item-hash allowlist. Adapter 7 preserves the previously tested SMG contract. Adapter 10 derives a gun's private rig and native grouped firing layout from its source data. Adapter 11 admits finite uncharged 2-64-round bursts with matching native count routes, digital primary input and ordinary magazine reloads. Charged triggers, continuous beams, swords and unresolved special layouts remain unsupported; this branch does not yet provide universal weapon support.

This is development work. Automated checks and local package builds are distinct from in-game validation; a newly generated donor must pass the gameplay checklist below before release.

## Using it

1. Install the matching Dawn runtime build into a separate game installation and keep its capability file alongside it.
2. Create a weapon in Parhelion and enable **Gameplay > Akimbo**. Set **Weapon gap** from 50 to 800 mm; **Reset** restores 360 mm. The default **Reload > Lower magazine; both when empty** reloads one hand at a time with R, choosing the hand with fewer rounds. Both reload together only when both are empty. Rebuild and reinstall after changing these controls.
3. Build and stage the complete recipe selection. Unsupported combinations fail with the component or asset requirement that could not be met.
4. Close the game and install the staged generation through the normal review and backup transaction.
5. Relaunch and reclaim the authored weapon from its category page in Collections. Use the ordinary category for the selected weapon family; for SMGs, use **Weapons > Primary > Submachine Guns**. The Dawn or Sunrise badge allows inspection only.

Left and right mouse buttons fire the two hands. Holding both alternates direct shots or complete bursts. A finite burst stays with its initiating hand, and paid queued rounds retain that ownership through reload interruption. Each hand has its own magazine, recovery and reload state. New recipes use **Lower magazine; both when empty**: pressing R reloads the hand with fewer rounds, or both when both are empty. The other hand remains available to fire. Existing **All non-full magazines** and **Empty magazines only** recipes retain their behavior and can be changed in the Reload selector. Aiming down sights is disabled. Both hands use one authored weapon design; separate offhand recipes are reserved and rejected. Native moving-part animation still follows the last firing hand rather than providing two complete animation clocks. For generated profiles, the companion samples the supported native tactical or empty reload-duration property for each hand at reload start; it does not impose the legacy fixed two-second duration. This adapter accepts magazine capacities from 1 to 200. Unsupported timing properties refuse reload; arbitrary perk interactions still require gameplay validation.

## Recipe contract

The optional `overrides.akimbo` value is:

```json
{
  "schema": 1,
  "mode": "independent",
  "reload": "lower_magazine",
  "gap_mm": 240
}
```

Omission preserves the normal weapon path. `gap_mm` is an integer in the inclusive range 50–800; omitting it retains the original 360 mm default. The default is omitted when saving, preserving the serialization of existing akimbo recipes. Intent, including a custom gap, survives save/load, duplication, renaming and donor changes, and participates in the recipe fingerprint. The compiler reassesses the resulting design on every build; recipes cannot assert their own compatibility. Unknown fields, modes, reload policies and intent versions are rejected. A non-null `offhand` is rejected until native mixed-pair ownership is implemented. Existing v0.6 readers reject the new field through their strict recipe parser.

`reload` is required. `lower_magazine` selects the hand with fewer rounds on R, or both when both are empty. `non_full_hands` selects every eligible non-full magazine independently. `empty_hands` selects empty magazines only. Historical values are not aliases for the new behavior. The selected policy is saved, fingerprinted and compiled into each native profile.

With `lower_magazine`, equal nonzero counts select the left hand. Pressing R during an active reload never restarts its timer. It adds the other hand only if both magazines have become empty; otherwise that hand waits for the current reload to finish.

## Current compatibility boundary

The compiler examines the final entity and its pending private component owners, rather than relying on an earlier stock donor assessment. Adapter 7 requires:

- A single input, trigger, controller, barrel, magazine and reload resource with the audited ownership, concrete classes and relative layouts.
- Direct single-round emission, with no charge component, spread pattern or delayed emission.
- An unambiguous selected content block, with no separate projectile/persistent effect override.
- The audited SMG animation, skeleton and arms rig. Matching only a bone count is insufficient.
- Native rigid weapon models with supported vertex/index formats, an available copy marker, bounded geometry, and recognizable native vertex shaders.
- Derivable muzzle/ejection frames, tracer identity and bounded local effect selectors. Unsupported multi-marker/particle-capacity layouts fail the build.

Adapter 10 adds source-derived combined arm/weapon rigs, including variable weapon FK prefixes and a private marker inserted before the trailing procedural range. Both combined and weapon-only hierarchies, control references, native attachment bone indices, model palettes and inspect graphs are remapped together. The original frozen animation header remains unchanged; native allocation uses the extended skeleton counts.

The generic gun route preserves native grouped emission and pellet patterns instead of multiplying projectile calls. It validates typed source patterns and bounds their origins to 64. Live bullet-count and ammunition-cost checks still require a direct unit-cost event. Ordinary and tube magazines share the requested hand-selection policy; tube reloads use the source opening, insertion quantum and closing timing. Animation-dependent timing that cannot be sampled remains unsupported.

Firing effects may come from the selected content's direct entity or its typed sequence-override wrapper. Additional local marker groups retain source order and native allocation capacity. Availability indices and bone indices are separate fields. A capacity-one selector may use the first of several source markers when all have the same availability index. Root-mounted attachments retain their local frames. Attachments on other weapon bones also record the renumbered bone and its object-space bind frame; the runtime samples the completed native bone palette and uses the same moving-hand mask as the shader. Effects and tracer origins follow the live bone on the moving hand and the source bind frame on the frozen hand. Independently changing visibility remains unsupported by this route. Native frame-count and owner checks remain mandatory.

Appearance swaps, imported models, animation overrides, additional behavior grafts, variable damage and moved hand/marker overrides currently require another adapter and are rejected when combined with akimbo. Shader Glow is composed with the paired vertex transform in the same private material. Other resolved edits must still satisfy the checked native interfaces.

The classifier checks structure, not every possible perk interaction. Weapon-global perk state remains weapon-global; independent triggers and magazines do not imply independent copies of all exotic state machines. This is one reason gameplay validation remains required.

See [the asset pipeline](AKIMBO-ASSETS.md) for geometry, shaders and effect dispatch.

## Native contract

The host is integrated into Dawn, rather than injected by Sundial. Sundial reads `Dawn/akimbo/runtime-capability.json` and hashes the installed runtime and executable without loading the DLL. This source requires descriptor schema 5, game build 86657, the runtime hash and explicitly advertised adapter IDs. The runtime must advertise adapter 7 for the established SMG contract, 10 for direct guns, and 11 for finite bursts as required by the selection. Earlier hosts or descriptors require a matching runtime and rebuilt generation. The current 33-weapon checkpoint uses schema 5; the accepted four-SMG HUD/reload checkpoint used schema 4. See [checkpoint status](AKIMBO-STATUS.md) for the validation boundary.

The generated `packages/parhelion-akimbo-bindings.json` contains a generation ID, executable/runtime SHA-256 pins, generated package hashes, stock dependency package hashes and one profile per akimbo recipe. Each profile carries:

- A private model-owner tag used as the native identity carrier.
- Adapter and exact animation/skeleton/rig identities.
- The same `gap_mm` used to compile the geometry and vertex programs.
- `reload_policy`: 1 for empty-only selection, 2 for all non-full magazines independently, or 3 for the lower magazine and both when both are empty.
- Core/content identities, definitions, sizes and selected content variant.
- Muzzle/ejection frame bits, tracer binding and typed local effect selectors.
- For adapters 10 and 11, exact weapon/combined-rig counts, marker placement, frozen animation identity, additional local marker groups and sparse attachment-bone bindings when needed.
- For adapter 11, a private generated controller core whose emitted ownership and generated-entry domain are checked before presentation initialization.

These are asset identities and checked layouts, not process addresses. Source package pins cover canonical investment inputs and visited/native-referenced package chains. They are not a claim to hash every file in the game installation.

The native registry is bounded and published once before gameplay initialization. The loader rejects malformed/unknown data, mismatched hashes, missing dependencies, unsupported adapters, ambiguous entries and pending package transactions. Runtime routes retain component self/owner checks, bounded walks and generation rechecks. An invalid descriptor prevents activation rather than falling back to a different weapon profile. Without a descriptor, the clean host can start normally.

One generated model owner identifies each weapon assembly. All its visible parts can be paired, while only that owner is registered. Native identification walks the assembly's model slots, requires exactly one registered carrier with the selected controller owner, and rejects competing registered carriers.

The generated pose centers the pair on the reticle's vertical axis before independent hand recoil and reload. Geometry keeps the original copy at local Y=0 and the left copy at Y=gap. The native pose shifts the right gun and arm by −gap/2 and applies a common translation that cancels the original root's camera-space lateral offset. It preserves vertical and depth animation. The left marker omits the baked gap; muzzle, ejection and tracer roots add it exactly once. Schema 5 preserves this pose contract. The user accepted the earlier SMG reload and HUD behavior, including steady counter updates. The generic gun expansion still needs in-game validation.

## Installation and recovery

The build captures the target's capability before compilation and verifies it again before staging. Descriptor and package pins are validated before target mutation. The existing package transaction installs the descriptor last, includes it in backups/recovery/uninstall, and removes it when replacing an akimbo generation with a normal generation. A descriptor cannot remain active against a partially replaced package set.

Installed schemas 1 through 4 are recognized only for backup, replacement or removal. The installer checks each original strict structure, preserves its exact bytes in the backup, and can restore those bytes after an interrupted upgrade. Schema 1 has no gap or reload policy; schema 2 requires the gap and has no reload policy. Both used empty-only selection. Schema 3 requires the gap and supports only reload policies 1 and 2; schema 4 adds policy 3. These historical schemas cannot contain schema-5 pose, additional-marker or attachment-bone fields. The installer does not activate or rewrite an old descriptor as schema 5. Unknown or malformed historical files remain protected; new staged bindings must use schema 5 and the matching runtime.

The native host checks the transaction journal even when the descriptor is absent. It retains read handles for its validated files while the generation is active. Close the game before installing another generation.

Sundial does not replace the native runtime or edit a save merely because a recipe enables akimbo. Runtime deployment is a separate, explicit installation step. Account and Collections actions use Sundial's existing workflows.

## Validation and release gate

The source includes synthetic geometry/shader/visual tests, strict recipe tests, native descriptor fixtures, and installer tests for capability changes, hash mismatch, recovery and uninstall. Optional stock-data tests are ignored by default and require local legally obtained game packages. No proprietary package, shader, animation, save or replay binary is included in the PR.

Local development also compares generated shaders with the original IKELOS transforms and exercises both hand branches using software GPU execution. The extracted native regression suite preserves the existing IKELOS replay and magazine behavior, alongside generated-tag/visual/ownership tests.

Before release, test a previously unlisted compatible donor without adding a donor-specific native condition:

- Equip/draw; inspect both models, textures and moving parts. Rebuild at 50, 360 and 800 mm and check the pair's lateral center and muzzle/tracer alignment at each gap.
- Fire left, right and both triggers; confirm cadence, damage and recoil. For finite bursts, confirm that every round stays in the initiating hand, both held inputs alternate whole bursts, and the other hand resumes after either input is released.
- Partially empty either or both magazines, then press R. Check that only the lower magazine reloads, unless both are empty. Check each magazine and its native HUD counter through firing, reload start/completion, weapon storage relocation and low reserves. Repeat with the historical all-non-full and empty-only policies.
- Check muzzle flash, shell ejection, tracer origin, cloak/outline and glow.
- Swap weapons, die, respawn and change activity; check stale state is discarded.
- Remove/reinstall the generation and verify normal weapons and the baseline still work.

Unsupported families remain rejected until their adapters and equivalent tests exist. Historical hand-cannon, Rat King and Sturm/Drang implementations are useful source evidence, but are not advertised as general authoring capabilities. The recorded First Curse equip crash and mixed-pair ownership limitations must not be carried into an implicit fallback.
