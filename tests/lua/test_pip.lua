-- Unit tests for scripts/local/pip.lua.
--
-- Run with: luajit tests/lua/test_pip.lua <path-to-script>
-- The mpv API is replaced by a stub that keeps native property values, records
-- observed properties, timers, bindings and OSD messages, and can simulate
-- set_property failures.  The real script is loaded unchanged, so these tests
-- exercise the same window math and rollback paths used inside mpv.

local script_path = arg[1]
if not script_path or script_path == "" then
	io.stderr:write("usage: luajit test_pip.lua <script.lua>\n")
	os.exit(2)
end

local TEST_OPTIONS = {}

local DEFAULT_PROPERTIES = {
	["display-width"] = 1920,
	["display-height"] = 1080,
	geometry = "",
	autofit = "",
	["autofit-larger"] = "",
	["autofit-smaller"] = "",
	["window-scale"] = 1.0,
	fullscreen = false,
	border = true,
	ontop = false,
	["keepaspect-window"] = true,
	["window-maximized"] = false,
	["video-params/dw"] = 1920,
	["video-params/dh"] = 1080,
	["video-params/w"] = 1920,
	["video-params/h"] = 1080,
	width = 1920,
	height = 1080,
	["osd-width"] = 960,
	["osd-height"] = 540,
}

local PROPERTIES = {}
local FAIL_PROPERTIES = {}

local function reset_properties()
	for key in pairs(PROPERTIES) do PROPERTIES[key] = nil end
	for key, value in pairs(DEFAULT_PROPERTIES) do PROPERTIES[key] = value end
end
reset_properties()

local state = {
	bindings = {},
	key_binding_names = {},
	message_names = {},
	messages = {},
	observers = {},
	events = {},
	timers = {},
	osd = {},
	changes = {},
	warnings = {},
	errors = {},
	option_errors = {},
}

local function add_timer(delay, fn)
	local timer = { delay = delay, fn = fn, killed = false }
	function timer:kill()
		self.killed = true
	end
	state.timers[#state.timers + 1] = timer
	return timer
end

local function stub_set_property(name, value)
	state.changes[#state.changes + 1] = { name = name, value = value }
	if FAIL_PROPERTIES[name] then
		error("simulated failure for " .. tostring(name))
	end
	PROPERTIES[name] = value
	return true
end

local mp_stub = {
	get_property = function(name, default)
		local value = PROPERTIES[name]
		if value == nil then return default end
		if type(value) == "boolean" then return value and "yes" or "no" end
		return tostring(value)
	end,
	get_property_native = function(name, default)
		local value = PROPERTIES[name]
		if value == nil then return default end
		return value
	end,
	get_property_number = function(name, default)
		local value = PROPERTIES[name]
		if type(value) == "number" then return value end
		if type(value) == "string" then return tonumber(value) or default end
		return default
	end,
	get_property_bool = function(name, default)
		local value = PROPERTIES[name]
		if type(value) == "boolean" then return value end
		if type(value) == "number" then return value ~= 0 end
		if type(value) == "string" then
			local text = value:lower()
			if text == "yes" or text == "true" or text == "1" then return true end
			if text == "no" or text == "false" or text == "0" then return false end
		end
		return default
	end,
	set_property = stub_set_property,
	set_property_native = stub_set_property,
	osd_message = function(text, duration)
		state.osd[#state.osd + 1] = { text = text, duration = duration }
	end,
	add_timeout = add_timer,
	add_key_binding = function(key, name, fn)
		state.bindings[#state.bindings + 1] = { key = key, name = name, fn = fn }
		state.key_binding_names[#state.key_binding_names + 1] = name
	end,
	register_script_message = function(name, fn)
		state.message_names[#state.message_names + 1] = name
		state.messages[name] = fn
	end,
	observe_property = function(name, kind, fn)
		state.observers[name] = { kind = kind, fn = fn }
	end,
	unobserve_property = function() return true end,
	register_event = function(name, fn)
		state.events[name] = fn
	end,
	msg = {
		info = function() end,
		warn = function(text)
			state.warnings[#state.warnings + 1] = tostring(text)
		end,
		error = function(text)
			state.errors[#state.errors + 1] = tostring(text)
		end,
		verbose = function() end,
		debug = function() end,
	},
}

local options_stub = {
	read_options = function(options)
		for key, raw in pairs(TEST_OPTIONS) do
			local default = options[key]
			if default == nil then
				options[key] = raw
			elseif type(default) == "boolean" then
				local text = tostring(raw):lower()
				if text == "yes" or text == "true" or text == "1" then
					options[key] = true
				elseif text == "no" or text == "false" or text == "0" then
					options[key] = false
				else
					state.option_errors[#state.option_errors + 1] = key
				end
			elseif type(default) == "number" then
				local number = tonumber(raw)
				if number == nil then
					state.option_errors[#state.option_errors + 1] = key
				else
					options[key] = number
				end
			else
				options[key] = tostring(raw)
			end
		end
		return options
	end,
}

package.preload["mp"] = function() return mp_stub end
package.preload["mp.msg"] = function() return mp_stub.msg end
package.preload["mp.options"] = function() return options_stub end

-- Loading must never open a file.  dofile() uses the C file API, so replacing
-- the Lua io.open function can safely detect script-level file access.
local real_open = io.open
io.open = function(...)
	error("pip.lua accessed a file while loading: " .. tostring((...)))
end
local ok, M = pcall(dofile, script_path)
io.open = real_open
if not ok then
	io.stderr:write("failed to load pip.lua: " .. tostring(M) .. "\n")
	os.exit(1)
end

local failures = 0
local function fail(message)
	failures = failures + 1
	io.stderr:write("FAIL: " .. message .. "\n")
end

local function ok(condition, message)
	if not condition then fail(message) end
end

local function eq(actual, expected, message)
	if actual ~= expected then
		fail(string.format("%s: expected %s, got %s", message,
			tostring(expected), tostring(actual)))
	end
end

local function near(actual, expected, message)
	if type(actual) ~= "number" or math.abs(actual - expected) > 0.0001 then
		fail(string.format("%s: expected %.4f, got %s", message,
			expected, tostring(actual)))
	end
end

local function contains(list, value, message)
	if type(list) ~= "table" then fail(message .. ": not a table") return end
	for _, item in ipairs(list) do
		if item == value then return end
	end
	fail(message .. ": missing " .. tostring(value))
end

local function sorted(list)
	local copy = {}
	for index, item in ipairs(list or {}) do copy[index] = item end
	table.sort(copy)
	return copy
end

local function same_names(actual, expected, label)
	local left = sorted(actual)
	local right = sorted(expected)
	eq(#left, #right, label .. " count")
	for index = 1, #right do
		eq(left[index], right[index], label .. "[" .. index .. "]")
	end
end

-- ---------------------------------------------------------------------------
-- Behaviour helpers
-- ---------------------------------------------------------------------------

local function reset_world()
	reset_properties()
	state.osd = {}
	state.changes = {}
	state.warnings = {}
	state.errors = {}
	state.timers = {}
	for key in pairs(FAIL_PROPERTIES) do FAIL_PROPERTIES[key] = nil end
end

local function run_timers()
	local guard = 0
	while true do
		local pending = nil
		for _, timer in ipairs(state.timers) do
			if not timer.killed then
				pending = timer
				break
			end
		end
		if pending == nil then break end
		pending.killed = true
		guard = guard + 1
		if guard > 50 then
			error("timer chain did not settle")
		end
		pending.fn()
	end
	state.timers = {}
end

local function fire(name, value)
	PROPERTIES[name] = value
	local observer = state.observers[name]
	if observer ~= nil then
		observer.fn(name, value)
	end
end

local function send(name, ...)
	local handler = state.messages[name]
	if handler == nil then
		error("script message is not registered: " .. tostring(name))
	end
	return handler(...)
end

local function last_osd()
	local entry = state.osd[#state.osd]
	if entry == nil then return "" end
	return tostring(entry.text)
end

local function change_index(name, value)
	for index, item in ipairs(state.changes) do
		if item.name == name and item.value == value then return index end
	end
	return nil
end

local function config(overrides)
	local result = {
		position = "bottom-right",
		width_percent = 25,
		height_percent = 0,
		margin = 16,
		min_width = 160,
	}
	for key, value in pairs(overrides or {}) do result[key] = value end
	return result
end

-- ---------------------------------------------------------------------------
-- Load behaviour and public interface
-- ---------------------------------------------------------------------------

ok(type(M) == "table", "script must return a helper table")
if type(M) ~= "table" then os.exit(1) end

eq(#state.osd, 0, "no OSD may be shown at load time")
eq(#state.changes, 0, "no property may be changed at load time")
eq(#state.timers, 0, "no timer may be created at load time")

local expected_commands = { "toggle", "on", "off", "status" }
same_names(state.message_names, expected_commands, "script messages")
same_names(M.bindings, expected_commands, "exported command list")

local expected_bindings = { "toggle", "on", "off", "status", "toggle-key" }
same_names(state.key_binding_names, expected_bindings, "key bindings")

local function binding_entry(name)
	for _, item in ipairs(state.bindings) do
		if item.name == name then return item end
	end
	return nil
end
for _, name in ipairs(expected_commands) do
	local entry = binding_entry(name)
	ok(entry ~= nil, "named binding is missing: " .. name)
	if entry ~= nil then
		eq(entry.key, nil, "named binding must not grab a key: " .. name)
	end
end
local toggle_key = binding_entry("toggle-key")
ok(toggle_key ~= nil, "configured toggle key binding is missing")
if toggle_key ~= nil then
	eq(toggle_key.key, "alt+p", "default toggle key")
end

eq(M.settings.enabled, true, "default enabled")
eq(M.settings.position, "bottom-right", "default position")
eq(M.settings.width_percent, 25, "default width_percent")
eq(M.settings.height_percent, 0, "default height_percent")
eq(M.settings.margin, 16, "default margin")
eq(M.settings.min_width, 160, "default min_width")
eq(M.settings.ontop, true, "default ontop")
eq(M.settings.border, false, "default border")
eq(M.settings.exit_fullscreen, true, "default exit_fullscreen")
eq(M.settings.follow_display, true, "default follow_display")
eq(M.settings.keep_on_new_file, true, "default keep_on_new_file")
eq(M.settings.toggle_keybind, "alt+p", "default toggle_keybind")

-- The fullscreen observer must be present for the exit-fullscreen handling.
ok(state.observers["display-width"] ~= nil, "display-width is not observed")
ok(state.observers["display-height"] ~= nil, "display-height is not observed")
ok(state.observers["fullscreen"] ~= nil, "fullscreen is not observed")
ok(state.events["file-loaded"] ~= nil, "file-loaded is not handled")

-- Static constraints: no shell, no subprocesses, original attribution.
local file = real_open(script_path, "rb")
ok(file ~= nil, "script file must be readable")
if file ~= nil then
	local text = file:read("*a")
	file:close()
	ok(text:find("newly authored for mpv_PlayKit", 1, true) ~= nil,
		"script must carry the original-work note")
	eq(text:find("os.execute", 1, true), nil, "os.execute must not be used")
	eq(text:find("io.popen", 1, true), nil, "io.popen must not be used")
	eq(text:find('name%s*=%s*"subprocess"'), nil, "no subprocess may be started")
	eq(text:find("mp.command_native", 1, true), nil,
		"pip must not run external commands")
end

-- ---------------------------------------------------------------------------
-- Configuration validation
-- ---------------------------------------------------------------------------

local warnings = {}
local validated = M.validate_settings({
	enabled = "maybe",
	position = "middle",
	width_percent = 150,
	height_percent = -5,
	margin = -4,
	min_width = -1,
	ontop = "maybe",
	border = "maybe",
	exit_fullscreen = "no",
	follow_display = "off",
	keep_on_new_file = nil,
	toggle_keybind = "  alt+p  ",
}, function(text) warnings[#warnings + 1] = text end)

eq(validated.enabled, true, "invalid enabled falls back to yes")
eq(validated.position, "bottom-right", "invalid position falls back")
eq(validated.width_percent, 25, "invalid width_percent falls back")
eq(validated.height_percent, 0, "invalid height_percent falls back")
eq(validated.margin, 16, "invalid margin falls back")
eq(validated.min_width, 160, "invalid min_width falls back")
eq(validated.ontop, true, "invalid ontop falls back")
eq(validated.border, false, "invalid border falls back")
eq(validated.exit_fullscreen, false, "false exit_fullscreen is kept")
eq(validated.follow_display, false, "off follow_display is kept")
eq(validated.keep_on_new_file, true, "missing keep_on_new_file falls back")
eq(validated.toggle_keybind, "alt+p", "toggle_keybind is trimmed")
ok(#warnings >= 7, "all invalid options must warn: got " .. #warnings)

validated = M.validate_settings({
	enabled = "no",
	position = "TOP-LEFT",
	width_percent = "40",
	height_percent = 20,
	margin = 8,
	min_width = 200,
	ontop = "no",
	border = "yes",
	exit_fullscreen = 0,
	follow_display = 1,
	keep_on_new_file = "false",
	toggle_keybind = "",
}, function() end)
eq(validated.enabled, false, "no disables the script")
eq(validated.position, "top-left", "position is case-insensitive")
eq(validated.width_percent, 40, "numeric string width_percent")
eq(validated.height_percent, 20, "height_percent is kept")
eq(validated.margin, 8, "margin is kept")
eq(validated.min_width, 200, "min_width is kept")
eq(validated.ontop, false, "ontop=no")
eq(validated.border, true, "border=yes")
eq(validated.exit_fullscreen, false, "0 is false")
eq(validated.follow_display, true, "1 is true")
eq(validated.keep_on_new_file, false, "keep_on_new_file=false")
eq(validated.toggle_keybind, "", "empty toggle key disables the key binding")

-- ---------------------------------------------------------------------------
-- Geometry math
-- ---------------------------------------------------------------------------

near(M.positive_ratio(16, 9), 16 / 9, "positive ratio")
eq(M.positive_ratio(0, 9), nil, "zero width has no ratio")
eq(M.positive_ratio("16", 9), nil, "non-number input has no ratio")

local target, target_err = M.compute_geometry(config(), 1920, 1080, 16 / 9)
ok(target ~= nil, "geometry should be computable: " .. tostring(target_err))
if target ~= nil then
	eq(target.w, 480, "25% of 1920")
	eq(target.h, 270, "16:9 height")
	eq(target.x, 1424, "bottom-right x")
	eq(target.y, 794, "bottom-right y")
	eq(M.format_geometry(target), "480x270+1424+794", "geometry string")
end

local position_cases = {
	{ "top-left", 16, 16 },
	{ "top-right", 1424, 16 },
	{ "bottom-left", 16, 794 },
	{ "bottom-right", 1424, 794 },
}
for _, case in ipairs(position_cases) do
	local result = M.compute_geometry(config({ position = case[1] }), 1920, 1080, 16 / 9)
	ok(result ~= nil, "geometry for " .. case[1])
	if result ~= nil then
		eq(result.x, case[2], case[1] .. " x")
		eq(result.y, case[3], case[1] .. " y")
	end
end

target = M.compute_geometry(config({ aspect = nil }), 1920, 1080, nil)
ok(target ~= nil, "nil aspect uses the display ratio")
if target ~= nil then
	eq(target.w, 480, "display-ratio width")
	eq(target.h, 270, "display-ratio height")
end

target = M.compute_geometry(
	config({ height_percent = 25, position = "bottom-right" }),
	1920, 1080, 4 / 3
)
ok(target ~= nil, "explicit height_percent")
if target ~= nil then
	eq(target.w, 480, "explicit height keeps width")
	eq(target.h, 270, "25% of 1080")
	eq(target.y, 794, "explicit height bottom y")
end

target = M.compute_geometry(config({ position = "bottom-right" }), 1920, 1080, 4 / 3)
ok(target ~= nil, "4:3 auto height")
if target ~= nil then
	eq(target.h, 360, "4:3 auto height")
	eq(target.y, 704, "4:3 bottom y")
end

target = M.compute_geometry(
	config({ width_percent = 1, min_width = 160 }),
	1920, 1080, 16 / 9
)
ok(target ~= nil, "min_width geometry")
if target ~= nil then
	eq(target.w, 160, "min_width wins over 1%")
	eq(target.h, 90, "min_width auto height")
	eq(target.x, 1744, "min_width x")
end

target = M.compute_geometry(
	config({ width_percent = 25, min_width = 160 }),
	100, 100, 1
)
ok(target ~= nil, "tiny display geometry")
if target ~= nil then
	eq(target.w, 100, "width is clamped to the display")
	eq(target.h, 100, "height is clamped to the display")
	eq(target.x, 0, "clamped x")
	eq(target.y, 0, "clamped y")
end

ok(M.compute_geometry(config(), 0, 1080, 16 / 9) == nil,
	"zero display width must fail")
ok(M.compute_geometry(config(), 1920, nil, 16 / 9) == nil,
	"missing display height must fail")

-- ---------------------------------------------------------------------------
-- Enter and exit restore every remembered property
-- ---------------------------------------------------------------------------

reset_world()
PROPERTIES.geometry = "800x450+10+20"
PROPERTIES.autofit = "50%x50%"
PROPERTIES["autofit-larger"] = "80%x80%"
PROPERTIES["autofit-smaller"] = "30%x30%"
PROPERTIES["window-scale"] = 1.25
PROPERTIES.fullscreen = false
PROPERTIES.border = true
PROPERTIES.ontop = false
PROPERTIES["keepaspect-window"] = false
PROPERTIES["window-maximized"] = true

send("on")
eq(PROPERTIES.geometry, "480x270+1424+794", "PiP geometry")
eq(PROPERTIES.border, false, "border is removed")
eq(PROPERTIES.ontop, true, "window is kept on top")
eq(PROPERTIES["window-maximized"], false, "window is unmaximized")
eq(PROPERTIES.autofit, "0x0", "autofit is cleared")
eq(PROPERTIES["autofit-larger"], "0x0", "autofit-larger is cleared")
eq(PROPERTIES["autofit-smaller"], "0x0", "autofit-smaller is cleared")
eq(PROPERTIES["window-scale"], 1.25, "window-scale is never touched")
eq(PROPERTIES["keepaspect-window"], false, "keepaspect-window is untouched")
ok(last_osd():find("已进入画中画", 1, true) ~= nil, "enter OSD")

send("status")
ok(last_osd():find("已开启", 1, true) ~= nil, "status reports active")
ok(last_osd():find("右下角", 1, true) ~= nil, "status reports the corner")

send("toggle")
eq(PROPERTIES.geometry, "800x450+10+20", "geometry restored")
eq(PROPERTIES.autofit, "50%x50%", "autofit restored")
eq(PROPERTIES["autofit-larger"], "80%x80%", "autofit-larger restored")
eq(PROPERTIES["autofit-smaller"], "30%x30%", "autofit-smaller restored")
eq(PROPERTIES["window-scale"], 1.25, "window-scale restored")
eq(PROPERTIES.fullscreen, false, "fullscreen restored")
eq(PROPERTIES.border, true, "border restored")
eq(PROPERTIES.ontop, false, "ontop restored")
eq(PROPERTIES["keepaspect-window"], false, "keepaspect-window restored")
eq(PROPERTIES["window-maximized"], true, "window-maximized restored")
ok(last_osd():find("已退出画中画", 1, true) ~= nil, "exit OSD")

send("status")
ok(last_osd():find("未开启", 1, true) ~= nil, "status reports inactive")
send("off")
ok(last_osd():find("未开启", 1, true) ~= nil, "off is idempotent")

-- Toggle twice more to prove the command itself is idempotent.
send("toggle")
eq(PROPERTIES.geometry, "480x270+1424+794", "toggle enters PiP")
send("toggle")
eq(PROPERTIES.geometry, "800x450+10+20", "toggle leaves PiP")

-- Unset/empty values must round-trip unchanged as well.
reset_world()
eq(PROPERTIES.geometry, "", "initial geometry is empty")
send("on")
eq(PROPERTIES.geometry, "480x270+1424+794", "empty geometry PiP")
send("off")
eq(PROPERTIES.geometry, "", "empty geometry restored")
eq(PROPERTIES.autofit, "", "empty autofit restored")
eq(PROPERTIES.border, true, "default border restored")

-- ---------------------------------------------------------------------------
-- Fullscreen handling
-- ---------------------------------------------------------------------------

reset_world()
PROPERTIES.fullscreen = true
send("on")
eq(PROPERTIES.fullscreen, false, "PiP leaves fullscreen")
eq(PROPERTIES.geometry, "480x270+1424+794", "PiP geometry after fullscreen")
local fullscreen_index = change_index("fullscreen", false)
local geometry_index = change_index("geometry", "480x270+1424+794")
ok(fullscreen_index ~= nil, "fullscreen must be changed")
ok(geometry_index ~= nil, "geometry must be changed")
if fullscreen_index ~= nil and geometry_index ~= nil then
	ok(fullscreen_index < geometry_index,
		"fullscreen must be left before applying the geometry")
end
run_timers()
eq(PROPERTIES.geometry, "480x270+1424+794", "settle reapply keeps geometry")
send("off")
eq(PROPERTIES.fullscreen, true, "fullscreen is restored")
eq(PROPERTIES.geometry, "", "geometry is restored after fullscreen")

-- Entering fullscreen while PiP is active and later leaving it reapplies PiP.
reset_world()
send("on")
PROPERTIES.geometry = "480x270+100+100"
fire("fullscreen", true)
eq(PROPERTIES.geometry, "480x270+100+100", "fullscreen keeps the window alone")
fire("fullscreen", false)
run_timers()
eq(PROPERTIES.geometry, "480x270+1424+794", "PiP is reapplied after fullscreen")
send("off")

-- The same, but the display changes while the user is in fullscreen.
reset_world()
send("on")
fire("fullscreen", true)
fire("display-width", 2560)
fire("display-height", 1440)
run_timers()
eq(PROPERTIES.geometry, "480x270+1424+794", "no resize while fullscreen")
fire("fullscreen", false)
run_timers()
eq(PROPERTIES.geometry, "640x360+1904+1064", "new display applied after fullscreen")
send("off")

-- Exiting PiP while the user is in fullscreen restores the windowed state.
reset_world()
send("on")
fire("fullscreen", true)
send("off")
eq(PROPERTIES.fullscreen, false, "fullscreen is left when it was not original")
eq(PROPERTIES.geometry, "", "geometry restored from fullscreen")
eq(PROPERTIES.border, true, "border restored from fullscreen")

-- ---------------------------------------------------------------------------
-- Failure handling
-- ---------------------------------------------------------------------------

reset_world()
PROPERTIES.geometry = "640x360+5+5"
PROPERTIES.autofit = "25%x25%"
PROPERTIES.border = true
PROPERTIES.ontop = false
PROPERTIES["window-maximized"] = false

FAIL_PROPERTIES.geometry = true
send("on")
eq(PROPERTIES.border, true, "border rolled back after failure")
eq(PROPERTIES.ontop, false, "ontop rolled back after failure")
eq(PROPERTIES.autofit, "25%x25%", "autofit rolled back after failure")
eq(PROPERTIES["window-maximized"], false, "window-maximized rolled back")
ok(last_osd():find("进入画中画失败", 1, true) ~= nil, "failure OSD")
send("status")
ok(last_osd():find("未开启", 1, true) ~= nil, "failed entry stays inactive")
FAIL_PROPERTIES.geometry = nil

reset_world()
PROPERTIES["display-width"] = 0
PROPERTIES["display-height"] = 0
send("on")
eq(last_osd(), "当前无法获取屏幕尺寸，画中画不可用",
	"missing display produces the documented OSD")
eq(#state.changes, 0, "no property may change without a display size")
send("status")
ok(last_osd():find("未开启", 1, true) ~= nil, "missing display keeps the state")

-- ---------------------------------------------------------------------------
-- New file, display changes and manual moves
-- ---------------------------------------------------------------------------

reset_world()
send("on")
eq(PROPERTIES.geometry, "480x270+1424+794", "initial PiP geometry")
PROPERTIES["video-params/dw"] = 1440
PROPERTIES["video-params/dh"] = 1080
PROPERTIES["video-params/w"] = 1440
PROPERTIES["video-params/h"] = 1080
PROPERTIES.width = 1440
PROPERTIES.height = 1080
state.events["file-loaded"]()
run_timers()
eq(PROPERTIES.geometry, "480x360+1424+704", "new file aspect is applied")

M.settings.keep_on_new_file = false
PROPERTIES["video-params/dw"] = 1920
PROPERTIES["video-params/dh"] = 1080
state.events["file-loaded"]()
run_timers()
eq(PROPERTIES.geometry, "480x360+1424+704", "keep_on_new_file=no keeps geometry")
M.settings.keep_on_new_file = true
send("off")

reset_world()
send("on")
fire("display-width", 2560)
fire("display-height", 1440)
run_timers()
eq(PROPERTIES.geometry, "640x360+1904+1064", "display change recomputes geometry")

M.settings.follow_display = false
fire("display-width", 1920)
fire("display-height", 1080)
run_timers()
eq(PROPERTIES.geometry, "640x360+1904+1064", "follow_display=no keeps geometry")
M.settings.follow_display = true
send("off")

reset_world()
send("on")
PROPERTIES.geometry = "480x270+100+100"
run_timers()
eq(PROPERTIES.geometry, "480x270+100+100", "a manual move is not reclaimed")
send("off")
eq(PROPERTIES.geometry, "", "manual-move state still restores")

if failures > 0 then
	io.stderr:write(string.format("%d test(s) failed\n", failures))
	os.exit(1)
end

print("pip Lua tests passed")
