# Akimbo development checkpoint

Reviewed against `KyleThmpsn/sundial` main on 2026-11-13. Upstream still points to
v0.6 commit `3f180dfbbe77101c77383c62eedb30e01bfdbccb`. The retained October 30
compiler checkpoint changes 83 source, documentation and fixture files and
includes the corrected private rig references. This update also corrects the
finite-burst documentation, applies Rust formatting and supplies the 33 recipes.

## What is implemented

- Same-design independent hands, per-hand magazines and native reload timing.
- Adapters 7 (established SMG), 10 (direct gun/tube reload) and 11 (finite
  uncharged digital bursts), selected by checked component layouts.
- Schema 5 runtime bindings, 50-800 mm spacing (360 mm default), and transactional
  package installation, recovery and retirement of schemas 1-4.
- Private paired rigid/weighted geometry, bounded shared-effect shader selection,
  centered poses, live muzzle/ejection placement and independent native HUD counts.
- Reclaimable ordinary Collections categories in addition to inspect-only badges.
- Retargeted validated interior rig references and reciprocal FK owner headers,
  correcting the package defect that crashed Nightshade at character selection.

The [33 recipe examples](recipes/akimbo/README.md) cover 11 families. Family
membership does not prove compatibility for every donor. Charged triggers,
continuous beams, swords, mixed offhand designs and unresolved special layouts
remain unsupported. Test-only charge code is a prototype, not a production adapter.

## Companion runtime

Sundial requires a separately built Dawn companion exposing schema 5 and the
required adapters. It does not install the companion DLL. The validated native
patch is based on Dawn v1.7.2 commit
`e801e23e22248dce35731a340395510ca73e8c72` and has SHA-256
`A08C9405BD60A42F58911BDA49A46DFED6307E540C155E6581EF926BAE6BCF31`.
The corresponding tested runtime SHA-256 is
`531D51EB1C10B7F3D66354793EF8D938E3F1A322585B052E844D21485592589F`.
These identify the retained development checkpoint; deployment checks the actual
installed executable/runtime and all selected dependencies.

Dawn production advanced by 33 commits to
`880dfbdd6b95ba50c5420f852ce8f5997af5dcbc` during this review. Its project file and
client activation/lifecycle files overlap the companion patch. The retained
v1.7.2 runtime validation does not establish compatibility with that newer base.

## Validation boundary

Fresh checks on 2026-11-13:

- Both retained patches apply cleanly to their declared bases with strict
  whitespace checking. All 83 compiler source files match their checkpoint and
  normalized patch output. All 107 native patch files match their export receipts.
- All 33 recipe hashes match the checkpoint, and the compiler/native schema 5
  descriptor fixtures are identical.
- `cargo fmt --all -- --check` passes after formatting three changed Rust files;
  `git diff --cached --check` also passes.
- The hash-verified retained Rust test executable passes 139 focused Akimbo
  tests, with 8 stock-data tests ignored. This is a rerun of the checkpoint binary;
  it is not a clean rebuild of the formatted source.
- All 20 synthetic native Akimbo suites compile and run successfully with MSVC
  C++20. Descriptor validation also runs with a 64 KiB stack. These checks use
  the companion sources, not a game process.

Retained October 30 evidence records the exact stock Nightshade regression,
corrected 33-weapon production build/readback, installed artifact verification,
and preservation of existing player data. Earlier companion evidence records
26 isolated DLL lifecycle cases and the real 33-profile package lifecycle.
Those installation and game-dependent checks were not repeated during this review.

The previously accepted SMG playtest confirms steady raised counters, red reload
digits, firing and lower-magazine reload behavior. The corrected expanded build
published all 33 profiles without a new crash report during a 154-second startup
observation, whose last recorded state was `bootflow:start`. **Character-selection
and gameplay acceptance of the corrected 33-weapon generation remain pending.**

The historical unfiltered library run failed in the UI texture lifecycle
(`Dropped TexturesDelta with 1 unapplied deltas`) and could abort while unwinding.
No passing full-library or full-CI result is claimed. Full repository CI and the
[gameplay checklist](AKIMBO-AUTHORING.md#validation-and-release-gate) remain release
gates. The samples contain recipes only; package assets and account data are not
part of this source update.
