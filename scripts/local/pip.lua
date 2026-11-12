-- pip.lua - original picture-in-picture window controller for mpv PlayKit
--
-- This script is newly authored for mpv_PlayKit; it does not import code from
-- the historical baseline scripts.  It shrinks the player window into a small
-- corner window, remembers every window-related property it touches, and
-- restores the exact previous values when picture-in-picture is turned off.
--
-- Loading the script only reads options and registers bindings.  It never
-- creates directories, writes files or starts subprocesses.

local mp = require "mp"
local msg = require "mp.msg"
local mp_options = require "mp.options"

local settings = {
	enabled = true,
	position = "bottom-right",
	width_percent = 25,
	height_percent = 0,
	margin = 16,
	min_width = 160,
	ontop = true,
	border = false,
	exit_fullscreen = true,
	follow_display = true,
	keep_on_new_file = true,
	toggle_keybind = "alt+p",
}
mp_options.read_options(settings, "pip")

-- ---------------------------------------------------------------------------
-- Pure helpers.  These are also exercised by tests/lua/test_pip.lua.
-- ---------------------------------------------------------------------------

local POSITIONS = {
	["bottom-right"] = true,
	["bottom-left"] = true,
	["top-right"] = true,
	["top-left"] = true,
}

local POSITION_LABELS = {
	["bottom-right"] = "右下角",
	["bottom-left"] = "左下角",
	["top-right"] = "右上角",
	["top-left"] = "左上角",
}

local function trim(value)
	return (tostring(value or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end

local function to_bool(value, fallback)
	local kind = type(value)
	if kind == "boolean" then return value end
	if kind == "number" then return value ~= 0 end
	local text = trim(value):lower()
	if text == "yes" or text == "true" or text == "1" or text == "on" then return true end
	if text == "no" or text == "false" or text == "0" or text == "off" then return false end
	return fallback
end

local function finite_number(value, fallback)
	local number = tonumber(value)
	if number == nil or number ~= number
		or number == math.huge or number == -math.huge then
		return fallback
	end
	return number
end

--- Normalize a raw option table.  Invalid values fall back to the documented
-- defaults and are reported through `warn`, so a broken pip.conf can never
-- prevent the script from loading.
local function validate_settings(source, warn)
	source = source or {}
	warn = warn or function(text) msg.warn("pip: " .. text) end
	local result = {}

	local enabled = to_bool(source.enabled, nil)
	if enabled == nil then
		warn("enabled 必须是 yes/no，已回退为 yes")
		enabled = true
	end
	result.enabled = enabled

	local position = trim(source.position):lower()
	if not POSITIONS[position] then
		warn("position 必须是 bottom-right、bottom-left、top-right 或 top-left，已回退为 bottom-right")
		position = "bottom-right"
	end
	result.position = position

	local width = finite_number(source.width_percent, 25)
	if width <= 0 or width > 100 then
		warn("width_percent 必须在 1 到 100 之间，已回退为 25")
		width = 25
	end
	result.width_percent = width

	local height = finite_number(source.height_percent, 0)
	if height < 0 or height > 100 then
		warn("height_percent 必须在 0 到 100 之间，已回退为 0")
		height = 0
	end
	result.height_percent = height

	local margin = finite_number(source.margin, 16)
	if margin < 0 then
		warn("margin 不能为负数，已回退为 16")
		margin = 16
	end
	result.margin = margin

	local min_width = finite_number(source.min_width, 160)
	if min_width < 0 then
		warn("min_width 不能为负数，已回退为 160")
		min_width = 160
	end
	result.min_width = math.floor(min_width + 0.5)

	local ontop = to_bool(source.ontop, nil)
	if ontop == nil then
		warn("ontop 必须是 yes/no，已回退为 yes")
		ontop = true
	end
	result.ontop = ontop

	local border = to_bool(source.border, nil)
	if border == nil then
		warn("border 必须是 yes/no，已回退为 no")
		border = false
	end
	result.border = border

	local exit_fullscreen = to_bool(source.exit_fullscreen, nil)
	if exit_fullscreen == nil then
		warn("exit_fullscreen 必须是 yes/no，已回退为 yes")
		exit_fullscreen = true
	end
	result.exit_fullscreen = exit_fullscreen

	local follow_display = to_bool(source.follow_display, nil)
	if follow_display == nil then
		warn("follow_display 必须是 yes/no，已回退为 yes")
		follow_display = true
	end
	result.follow_display = follow_display

	local keep_on_new_file = to_bool(source.keep_on_new_file, nil)
	if keep_on_new_file == nil then
		warn("keep_on_new_file 必须是 yes/no，已回退为 yes")
		keep_on_new_file = true
	end
	result.keep_on_new_file = keep_on_new_file

	result.toggle_keybind = trim(source.toggle_keybind)

	return result
end

settings = validate_settings(settings)

local function positive_ratio(width, height)
	if type(width) ~= "number" or type(height) ~= "number" then return nil end
	if width <= 0 or height <= 0 then return nil end
	return width / height
end

--- Compute the target PiP rectangle in absolute pixels.
-- `aspect` is the video (or window) width/height ratio used when
-- height_percent is 0.  Returns nil and a user-facing reason when the display
-- dimensions are not usable.
local function compute_geometry(config, display_width, display_height, aspect)
	if type(display_width) ~= "number" or type(display_height) ~= "number"
		or display_width <= 0 or display_height <= 0 then
		return nil, "当前无法获取屏幕尺寸，画中画不可用"
	end

	local width = math.floor(display_width * config.width_percent / 100 + 0.5)
	local minimum = math.max(0, math.floor(config.min_width + 0.5))
	if width < minimum then width = minimum end
	if width > display_width then width = display_width end
	if width < 1 then width = 1 end

	local height
	if config.height_percent > 0 then
		height = math.floor(display_height * config.height_percent / 100 + 0.5)
	else
		if type(aspect) ~= "number" or aspect <= 0 then
			aspect = display_width / display_height
		end
		height = math.floor(width / aspect + 0.5)
	end
	if height < 1 then height = 1 end
	if height > display_height then height = display_height end

	local margin = math.max(0, math.floor(config.margin + 0.5))
	local x = margin
	local y = margin
	if config.position == "bottom-right" or config.position == "bottom-left" then
		y = display_height - height - margin
	end
	if config.position == "bottom-right" or config.position == "top-right" then
		x = display_width - width - margin
	end
	if x < 0 then x = 0 end
	if y < 0 then y = 0 end
	if x + width > display_width then x = display_width - width end
	if y + height > display_height then y = display_height - height end
	if x < 0 then x = 0 end
	if y < 0 then y = 0 end

	return { x = x, y = y, w = width, h = height }
end

local function format_geometry(target)
	return string.format("%dx%d+%d+%d", target.w, target.h, target.x, target.y)
end

-- ---------------------------------------------------------------------------
-- Runtime helpers
-- ---------------------------------------------------------------------------

-- Every property is snapshotted with its native type: booleans stay booleans,
-- numbers stay numbers and empty strings stay empty strings.  mpv rejects
-- string values for some window flags, so restoring the typed value is the
-- only reliable round trip.
local SNAPSHOT_PROPS = {
	"geometry",
	"autofit",
	"autofit-larger",
	"autofit-smaller",
	"window-scale",
	"fullscreen",
	"border",
	"ontop",
	"keepaspect-window",
	"window-maximized",
}

-- fullscreen is restored last so the window does not enter fullscreen before
-- the other properties are back in place.
local RESTORE_ORDER = {
	"geometry",
	"autofit",
	"autofit-larger",
	"autofit-smaller",
	"window-scale",
	"window-maximized",
	"keepaspect-window",
	"border",
	"ontop",
	"fullscreen",
}

local state = {
	active = false,
	original = nil,
	target = nil,
	in_fullscreen = false,
	pending = nil,
}

local function osd(text, duration)
	mp.osd_message(tostring(text), duration or 3000)
end

local function display_size()
	local width = mp.get_property_number("display-width", 0) or 0
	local height = mp.get_property_number("display-height", 0) or 0
	if width <= 0 or height <= 0 then return nil end
	return width, height
end

--- Aspect ratio of the current video.
-- mpv exposes the display size as video-params/dw and video-params/dh; a few
-- spellings (dwidth/dheight) are accepted for compatibility with other
-- versions and test fixtures.  The raw size and the global width/height
-- properties are the documented fallbacks.
local function video_aspect()
	local pairs_to_try = {
		{ "video-params/dwidth", "video-params/dheight" },
		{ "video-params/dw", "video-params/dh" },
		{ "video-params/w", "video-params/h" },
		{ "width", "height" },
	}
	for _, names in ipairs(pairs_to_try) do
		local width = mp.get_property_number(names[1], 0) or 0
		local height = mp.get_property_number(names[2], 0) or 0
		local ratio = positive_ratio(width, height)
		if ratio then return ratio end
	end
	return nil
end

local function window_aspect()
	local width = mp.get_property_number("osd-width", 0) or 0
	local height = mp.get_property_number("osd-height", 0) or 0
	return positive_ratio(width, height)
end

local function snapshot_properties()
	local values = {}
	for _, name in ipairs(SNAPSHOT_PROPS) do
		local ok, value = pcall(mp.get_property_native, name)
		if ok then values[name] = value end
	end
	return values
end

local function cancel_pending()
	if state.pending ~= nil then
		state.pending:kill()
		state.pending = nil
	end
end

local function schedule(delay, fn)
	cancel_pending()
	state.pending = mp.add_timeout(delay, function()
		state.pending = nil
		if not state.active then return end
		fn()
	end)
end

--- Recompute and apply the geometry for the current display and video.
-- Called after file-loaded and display changes.  When the user is inside
-- fullscreen the new rectangle is only remembered; it is applied when
-- fullscreen ends.
local function apply_geometry()
	if not state.active then return false end
	local display_width, display_height = display_size()
	if not display_width then
		msg.warn("pip: display size is unavailable; keeping the current window")
		osd("无法获取屏幕尺寸，画中画窗口保持不变", 3500)
		return false
	end
	local aspect = video_aspect() or window_aspect()
		or (display_width / display_height)
	local target = compute_geometry(settings, display_width, display_height, aspect)
	if not target then return false end
	state.target = target

	if mp.get_property_bool("fullscreen", false) == true then
		state.in_fullscreen = true
		return true
	end

	local ok, err = pcall(mp.set_property_native, "geometry", format_geometry(target))
	if not ok then
		msg.warn("pip: could not update the window geometry: " .. tostring(err))
		osd("更新画中画窗口失败：" .. tostring(err), 4500)
		return false
	end
	return true
end

local function enter_pip()
	if state.active then
		osd("画中画已开启", 2000)
		return
	end
	local display_width, display_height = display_size()
	if not display_width then
		osd("当前无法获取屏幕尺寸，画中画不可用", 4000)
		return
	end
	local fullscreen = mp.get_property_bool("fullscreen", false) == true
	if fullscreen and not settings.exit_fullscreen then
		osd("当前处于全屏，请先退出全屏或启用 exit_fullscreen", 4000)
		return
	end
	local aspect = video_aspect() or window_aspect()
		or (display_width / display_height)
	local target = compute_geometry(settings, display_width, display_height, aspect)
	if not target then
		osd("当前无法获取屏幕尺寸，画中画不可用", 4000)
		return
	end

	local original = snapshot_properties()
	local changed = {}

	local function apply(name, value)
		local ok, err = pcall(mp.set_property_native, name, value)
		if not ok then return false, tostring(err) end
		changed[#changed + 1] = { name = name, value = original[name] }
		return true
	end

	local function rollback()
		for index = #changed, 1, -1 do
			local item = changed[index]
			if item.value ~= nil then
				local ok, err = pcall(mp.set_property_native, item.name, item.value)
				if not ok then
					msg.warn("pip: could not restore '" .. item.name
						.. "' after a failure: " .. tostring(err))
				end
			else
				msg.warn("pip: no saved value for '" .. item.name .. "'; cannot restore it")
			end
		end
		changed = {}
	end

	local function fail(detail)
		rollback()
		msg.error("pip: could not enter picture-in-picture: " .. tostring(detail))
		osd("进入画中画失败：" .. tostring(detail), 5000)
	end

	local exited_fullscreen = false
	if fullscreen then
		local ok, err = apply("fullscreen", false)
		if not ok then
			fail("无法退出全屏：" .. tostring(err))
			return
		end
		exited_fullscreen = true
	end

	local ok, err = apply("window-maximized", false)
	if not ok then
		fail("无法取消窗口最大化：" .. tostring(err))
		return
	end
	for _, name in ipairs({ "autofit", "autofit-larger", "autofit-smaller" }) do
		local applied, apply_err = apply(name, "0x0")
		if not applied then
			fail("无法清除自动适配：" .. tostring(apply_err))
			return
		end
	end
	ok, err = apply("border", settings.border)
	if not ok then
		fail("无法设置窗口边框：" .. tostring(err))
		return
	end
	ok, err = apply("ontop", settings.ontop)
	if not ok then
		fail("无法设置窗口置顶：" .. tostring(err))
		return
	end
	ok, err = apply("geometry", format_geometry(target))
	if not ok then
		fail("无法设置窗口位置：" .. tostring(err))
		return
	end

	state.active = true
	state.original = original
	state.target = target
	state.in_fullscreen = false

	osd(string.format(
		"已进入画中画（%s，%dx%d）",
		POSITION_LABELS[settings.position], target.w, target.h
	), 3000)

	-- Leaving fullscreen can change the reported display size once the window
	-- manager has settled; refresh the rectangle once shortly afterwards.
	if exited_fullscreen then
		schedule(0.25, apply_geometry)
	end
end

local function exit_pip()
	if not state.active then
		osd("画中画未开启", 2000)
		return
	end
	state.active = false
	state.in_fullscreen = false
	cancel_pending()

	local original = state.original or {}
	state.original = nil
	state.target = nil

	-- If the user entered fullscreen while PiP was active and fullscreen was
	-- not part of the original state, leave fullscreen first so the restored
	-- geometry is actually applied by the window manager.
	if mp.get_property_bool("fullscreen", false) == true
		and original.fullscreen ~= true then
		local ok, err = pcall(mp.set_property_native, "fullscreen", false)
		if not ok then
			msg.warn("pip: could not leave fullscreen: " .. tostring(err))
		end
	end

	local failures = 0
	for _, name in ipairs(RESTORE_ORDER) do
		local value = original[name]
		if value ~= nil then
			local ok, err = pcall(mp.set_property_native, name, value)
			if not ok then
				failures = failures + 1
				msg.warn("pip: could not restore '" .. name .. "': " .. tostring(err))
			end
		else
			msg.warn("pip: no saved value for '" .. name .. "'; leaving it unchanged")
		end
	end

	if failures > 0 then
		osd(string.format("已退出画中画，%d 项窗口设置未能恢复", failures), 4500)
	else
		osd("已退出画中画", 2500)
	end
end

local function toggle_pip()
	if state.active then
		exit_pip()
	else
		enter_pip()
	end
end

local function status()
	if state.active and state.target ~= nil then
		osd(string.format(
			"画中画：已开启，位置：%s，尺寸：%dx%d",
			POSITION_LABELS[settings.position],
			state.target.w, state.target.h
		), 3000)
	else
		osd("画中画：未开启", 2000)
	end
end

--- Reapply after rotating/resizing the display.  This is the only situation
-- where a manually moved window is put back into its PiP corner; plain window
-- moves are ignored by design.
local function on_display_change()
	if not state.active or not settings.follow_display then return end
	schedule(0.3, apply_geometry)
end

local function on_fullscreen_change(_, value)
	if not state.active then return end
	if value == true then
		state.in_fullscreen = true
	elseif state.in_fullscreen then
		state.in_fullscreen = false
		schedule(0.15, apply_geometry)
	end
end

local HANDLERS = {
	toggle = toggle_pip,
	on = enter_pip,
	off = exit_pip,
	status = status,
}

local BINDING_NAMES = { "toggle", "on", "off", "status" }

local M = {
	settings = settings,
	positions = POSITIONS,
	position_labels = POSITION_LABELS,
	bindings = BINDING_NAMES,
	snapshot_props = SNAPSHOT_PROPS,
	restore_order = RESTORE_ORDER,
	validate_settings = validate_settings,
	compute_geometry = compute_geometry,
	format_geometry = format_geometry,
	positive_ratio = positive_ratio,
}

if not settings.enabled then
	msg.info("pip is disabled by configuration")
	return M
end

-- Both named key bindings and script messages use the same command names, so
-- menu.conf and input.conf can call `script-binding pip/<name>` or
-- `script-message-to pip <name>`.
for _, name in ipairs(BINDING_NAMES) do
	mp.add_key_binding(nil, name, HANDLERS[name])
	mp.register_script_message(name, HANDLERS[name])
end
if settings.toggle_keybind ~= "" then
	local ok, err = pcall(
		mp.add_key_binding, settings.toggle_keybind, "toggle-key", HANDLERS.toggle
	)
	if not ok then
		msg.warn("pip: could not register toggle_keybind '"
			.. settings.toggle_keybind .. "': " .. tostring(err))
	end
end

mp.observe_property("display-width", "native", on_display_change)
mp.observe_property("display-height", "native", on_display_change)
mp.observe_property("fullscreen", "native", on_fullscreen_change)

mp.register_event("file-loaded", function()
	if not state.active or not settings.keep_on_new_file then return end
	schedule(0.2, apply_geometry)
end)

return M
