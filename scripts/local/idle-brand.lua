-- A small, portable idle-screen label for mpv_PlayKit.
-- Newly authored for this project; it does not reuse the baseline vector logo.

local mp = require "mp"
local options = require "mp.options"

local settings = {
	label = "mpv_PlayKit",
	font = "sans-serif",
	font_size = 34,
	color = "&H00FFFFFF&",
	outline_color = "&H80000000&",
}
options.read_options(settings)

local overlay = mp.create_osd_overlay("ass-events")
overlay.z = 10

local function escape_ass(text)
	return tostring(text or "")
		:gsub("\\", "\\\\")
		:gsub("{", "\\{")
		:gsub("}", "\\}")
end

local function update()
	if mp.get_property_native("idle-active") ~= true then
		overlay:remove()
		return
	end

	local dimensions = mp.get_property_native("osd-dimensions") or {}
	local width = tonumber(dimensions.w) or 1280
	local height = tonumber(dimensions.h) or 720
	local font_size = math.max(12, tonumber(settings.font_size) or 34)
	local font = escape_ass(settings.font)
	local label = escape_ass(settings.label)
	local color = tostring(settings.color)
	local outline = tostring(settings.outline_color)

	overlay.data = string.format(
		"{\\an5\\pos(%d,%d)\\fn%s\\fs%d\\bord1.5\\1c%s\\3c%s}%s",
		math.floor(width / 2),
		math.floor(height / 2),
		font,
		font_size,
		color,
		outline,
		label
	)
	overlay:update()
end

mp.observe_property("idle-active", "bool", update)
mp.observe_property("osd-dimensions", "native", update)
mp.register_event("file-loaded", update)
mp.register_event("end-file", update)
