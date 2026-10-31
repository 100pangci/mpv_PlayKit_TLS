"""Apply the mpv PlayKit UI-scale patch to the vendored uosc.

uosc derives its UI size from the HiDPI factor reported by the operating
system.  On a 125% display that renders every element 25% larger than at
100%, and the size only changes when the DPI factor changes.  The reference
player instead sizes its UI from the current display's shorter side
(1080p -> 100%) and recomputes it when the window moves to another display.

The patch adds a `scale=0` mode with that behaviour; any other value keeps
uosc's stock semantics.  uosc is licensed LGPL-2.1-or-later, so the modified
file carries a prominent notice with the change date, and the build fails
loudly if a future upstream release moves the anchor lines instead of
silently shipping an unpatched UI.
"""

from __future__ import annotations

from pathlib import Path

from .common import fail, log

MARKER = "-- Modified by mpv PlayKit"

SCALE_ANCHOR = (
    "\tstate.scale = (state.hidpi_scale or 1) "
    "* (state.fullormaxed and options.scale_fullscreen or options.scale)"
)

SCALE_PATCH = """\t-- Modified by mpv PlayKit on 2026-10-31: `scale=0` selects a factor
\t-- derived from the shorter side of the current display (1080p -> 100%)
\t-- instead of the OS HiDPI factor, and follows the window when it is moved
\t-- to another display.  Non-zero values keep upstream behaviour.
\tlocal user_scale = options.scale
\tif user_scale == 0 then
\t\tlocal dw = mp.get_property_number('display-width', 0) or 0
\t\tlocal dh = mp.get_property_number('display-height', 0) or 0
\t\tlocal shorter = math.min(dw, dh)
\t\tlocal factor = shorter > 0 and shorter / 1080 or 1
\t\tstate.scale = (tonumber(string.format('%.2f', factor)) or 1)
\t\t\t* (state.fullormaxed and options.scale_fullscreen or 1)
\telse
\t\tstate.scale = (state.hidpi_scale or 1) * (state.fullormaxed and options.scale_fullscreen or user_scale)
\tend"""

OBSERVER_ANCHOR = (
    "mp.observe_property('display-hidpi-scale', 'native', "
    "create_state_setter('hidpi_scale', update_display_dimensions))"
)

OBSERVER_PATCH = OBSERVER_ANCHOR + """
-- Modified by mpv PlayKit on 2026-10-31: recompute the automatic scale when
-- the window moves to a display with a different resolution.
mp.observe_property('display-width', 'native', update_display_dimensions)
mp.observe_property('display-height', 'native', update_display_dimensions)"""


def patch_uosc(stage: Path) -> None:
    path = stage / "portable_config" / "scripts" / "uosc" / "main.lua"
    if not path.is_file():
        fail(f"uosc main.lua is missing: {path}")
    text = path.read_text(encoding="utf-8")
    if MARKER in text:
        log("uosc UI-scale patch already applied")
        return
    for anchor, name in ((SCALE_ANCHOR, "scale"), (OBSERVER_ANCHOR, "display observer")):
        if text.count(anchor) != 1:
            fail(
                "uosc main.lua changed upstream: cannot find the "
                f"{name} anchor; update build/patch_uosc.py"
            )
    text = text.replace(SCALE_ANCHOR, SCALE_PATCH)
    text = text.replace(OBSERVER_ANCHOR, OBSERVER_PATCH)
    path.write_text(text, encoding="utf-8", newline="\n")
    log("patched uosc: automatic UI scale (scale=0)")
