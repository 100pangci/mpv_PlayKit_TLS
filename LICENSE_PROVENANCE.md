# License provenance record

This is a provenance note, **not a project-wide license grant**. No blanket license was selected for the imported PlayKit material.

## Pinned PlayKit source policy

- Repository: [`hooke007/mpv_PlayKit`](https://github.com/hooke007/mpv_PlayKit)
- Commit checked: `4921c6796620a13c2c03266f368d59d354a67b72`
- File: [`LICENSE.MD`](https://github.com/hooke007/mpv_PlayKit/blob/4921c6796620a13c2c03266f368d59d354a67b72/LICENSE.MD)
- Git blob SHA-1: `835c54d8c3f2c52048415eb52ab21eeb26d57897`
- SHA-256: `8e2f0edb344a9ca79bacff16567a9e6c45035dfa8e806ccf62427f758f9a1779`

The pinned file says unlisted files default to **UNLICENSED** and notes that portable scripts and shaders usually carry their terms in-file. This policy does not grant a new license to copied files. A missing license is not permission to redistribute.

## Lua findings (2026-10-29)

The 20260510 portable configuration contains 34 Lua files:

- 25 uosc files were mapped to commits in [`tomasklaen/uosc`](https://github.com/tomasklaen/uosc): 16 matched upstream blobs byte-for-byte and 9 were modified. The repository license metadata is LGPL-2.1; its `LICENSE.LGPL` blob is `4362b49151d7b34ef83b3067a8f9c9f877d72a0e` at both mapped commits `99510d50b89e3724b6115d9ef06731e97a50b7cf` and `41040532f840b8089ae1bedba906071959347771`.
- `uosc/lib/fzy.lua` contains an MIT notice.
- `uosc/lib/utils.lua` includes a thumbfast FFI snippet marked Mozilla Public License 2.0; its scope needs to be preserved separately from uosc's LGPL terms.
- The following 9 files remain unresolved under the pinned PlayKit policy or could not be mapped to those uosc commits:
  - `portable_config/scripts/uosc/elements/Logo.lua`
  - `portable_config/scripts/uosc/lib/lang.lua`
  - `portable_config/scripts/input_plus.lua`
  - `portable_config/scripts/save_global_props.lua`
  - `portable_config/scripts/thumb_engine/batch.lua`
  - `portable_config/scripts/thumb_engine/helper.lua`
  - `portable_config/scripts/thumb_engine/main.lua`
  - `portable_config/scripts/thumb_engine/process.lua`
  - `portable_config/scripts/thumb_engine/winapi.lua`

The file-level hashes, exact upstream matches, and pending Lua list were recorded in the now-removed working tree's `baseline/uosc-lua-license-audit.json` and `baseline/lua-license-review-pending.md`; the repository was not pushed.

## Shader findings (2026-10-29)

The initial catalog contained 429 GLSL files from the same pinned PlayKit commit. The static scan recorded 138 as unlicensed by the pinned policy, 79 with license references not verified against the exact transformed work, 31 with inherited terms not verified, and 181 with embedded terms whose scope still needs review. Twenty-six mutable GitHub source/license references were fetched as leads only; fetching a URL does not establish that its license applies to the copied or modified shader.

Some files contain separate code and model terms (for example, an MIT code notice alongside CC BY-NC 4.0 model weights). These must not be collapsed into one repository license.

## Conclusion

There is no single verified license covering all imported Lua/GLSL content. Preserve each upstream notice and license separately; obtain permission or exclude unresolved files before public redistribution. This record does not classify ordinary mpv option values or grant rights to any third-party work.

## Current repository disposition (2026-10-31)

None of the unresolved baseline files listed above are copied into this repository or
its build artifacts. Their functionality was reimplemented or replaced:

| Baseline file | Disposition |
| --- | --- |
| `uosc/elements/Logo.lua` | Replaced by the original `scripts/local/idle-brand.lua` |
| `uosc/lib/lang.lua` | Replaced by upstream uosc `lib/intl.lua` (LGPL-2.1) |
| `input_plus.lua` | Rewritten as `scripts/local/input_plus.lua` |
| `save_global_props.lua` | Rewritten as `scripts/local/save_global_props.lua` |
| `thumb_engine/{batch,helper,main,process,winapi}.lua` | Merged into the original `scripts/local/thumbfast.lua`; no `thumb_engine/` directory exists |

The idle badge in `scripts/local/idle-brand.lua` reproduces the visual design of
mpv's OSC idle logo. That design comes from mpv's `player/lua/osc.lua`, which is
part of mpv (GPL-2.0-or-later / LGPL-2.1-or-later), not from the unlicensed
baseline `Logo.lua`. The Lua drawing code is original; mpv's license texts are
shipped in `licenses/`.

uosc itself is downloaded from the upstream release at build time and then
receives one documented patch: `build/patch_uosc.py` adds the automatic
UI-scale mode (`scale=0`) to `scripts/uosc/main.lua`, with a prominent
modification notice and date in the file. All other uosc files stay upstream
verbatim, and the patched Lua source ships in every package. Its bundled
`lib/fzy.lua` (MIT) and the MPL-2.0 FFI snippet in `lib/utils.lua`
keep their in-file notices, and their full license texts are added to
`licenses/uosc/`. The repository's original code is licensed under
GPL-3.0-or-later; every third-party component remains under its own upstream
license, recorded in `sources.lock.json`, `THIRD_PARTY.md`, and the package's
`licenses/INDEX.json`. See `docs/lua-disposition.md` for the full mapping and
`NOTICE` for the third-party boundary and corresponding-source information.
