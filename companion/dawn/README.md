# Dawn akimbo companion

This is the native Dawn companion for [Sundial's akimbo development checkpoint](../../crates/parhelion/AKIMBO-STATUS.md). The patch adds 107 source, test and fixture files or changes to Dawn, including schema 5 bindings and adapters 7 (SMG), 10 (direct gun/tube reload), and 11 (finite uncharged bursts). Sundial does not install this runtime automatically.

## Exact base and patch

- Base: Dawn v1.7.2, commit `e801e23e22248dce35731a340395510ca73e8c72`.
- Patch: [native-dawn-v1.7.2-companion.patch](native-dawn-v1.7.2-companion.patch).
- Patch SHA-256: `A08C9405BD60A42F58911BDA49A46DFED6307E540C155E6581EF926BAE6BCF31`.
- Resulting Dawn Git tree: `511574207828e2ef9593c1813bb4b1430c66580e`.

Apply to the exact base in a separate checkout. Starting from a directory containing a clone of this repository named `sundialakimbo`:

```powershell
git clone https://github.com/isinternets/Dawn.git dawn-akimbo
git -C dawn-akimbo switch -c akimbo-v1.7.2 e801e23e22248dce35731a340395510ca73e8c72
git -C dawn-akimbo apply --check --whitespace=error-all ../sundialakimbo/companion/dawn/native-dawn-v1.7.2-companion.patch
git -C dawn-akimbo apply --whitespace=error-all ../sundialakimbo/companion/dawn/native-dawn-v1.7.2-companion.patch
```

## Build and validation

Follow the patched Dawn checkout's source-build instructions. This base requires MSBuild 18 Build Tools, the v145 C++ toolset, and the Windows SDK selected by `Dawn/Dawn.vcxproj`. Build that project directly in Release/x64; it produces `build/x64/Release/steam_api64.dll`. The native test commands are in `Dawn/unit/AKIMBO-TESTS.md` after applying the patch. No DLL or game assets are included here.

The 2026-11-13 review rebuilt and passed all 20 synthetic native suites using MSVC C++20 with `/O2 /W4 /WX`; the descriptor suite also ran with a 64 KiB stack. Publication checks verified the patch hash, strict application to the declared base, and exact agreement of all 107 files with the reviewed source. These tests do not establish gameplay acceptance.

This remains a development checkpoint. Compatibility with newer Dawn production is unverified, and the corrected 33-weapon generation still needs character-selection and gameplay acceptance. Charged triggers, beams, swords, mixed offhand designs and unresolved layouts remain unsupported. See the [checkpoint status](../../crates/parhelion/AKIMBO-STATUS.md) for the complete validation boundary.
