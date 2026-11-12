-- Unit tests for scripts/local/screenshot_tools.lua.
--
-- Run with: luajit tests/lua/test_screenshot_tools.lua <path-to-script>
-- The mpv API is replaced by a small stub so the real script logic, including
-- argument parsing and code generation, runs without a player instance.

local script_path = arg[1]
if not script_path or script_path == "" then
	io.stderr:write("usage: luajit test_screenshot_tools.lua <script.lua>\n")
	os.exit(2)
end

local CONFIG_ROOT = "/tmp/playkit-screenshot-tools/portable_config"
local DESKTOP_ROOT = "/tmp/playkit-screenshot-tools/desktop"
local CWD = "/tmp/playkit-screenshot-tools/cwd"
local TEST_OPTIONS = {}

local state = {
	binding_names = {},
	message_names = {},
	subprocess_calls = {},
	async_calls = {},
	osd = {},
	commands = {},
	timers = {},
}

local FS = {}

local function expand(value)
	local text = tostring(value or "")
	if text == "~~" or text == "~~/" then return CONFIG_ROOT end
	if text == "~~desktop/" then return DESKTOP_ROOT end
	if text:sub(1, 3) == "~~/" then
		return CONFIG_ROOT .. "/" .. text:sub(4)
	end
	return text
end

local mp_stub = {
	get_property = function(_, default) return default end,
	get_property_number = function(_, default) return default end,
	get_property_native = function(_, default) return default end,
	get_property_bool = function(_, default) return default end,
	set_property = function() return true end,
	get_time = function() return 0 end,
	get_opt = function() return nil end,
	getpid = function() return 1 end,
	osd_message = function(text, duration)
		state.osd[#state.osd + 1] = { text = text, duration = duration }
	end,
	command = function(...)
		state.commands[#state.commands + 1] = { ... }
	end,
	commandv = function(...)
		state.commands[#state.commands + 1] = { ... }
	end,
	command_native = function(command)
		if type(command) == "table" then
			if command[1] == "expand-path" then
				return expand(command[2])
			end
			if command.name == "subprocess" then
				state.subprocess_calls[#state.subprocess_calls + 1] = command
				return { status = 0, stdout = "", stderr = "" }
			end
		end
		return true
	end,
	command_native_async = function(command, callback)
		state.async_calls[#state.async_calls + 1] = { command = command, callback = callback }
		return { abort = function() end }
	end,
	abort_async_command = function() return true end,
	add_timeout = function(delay, fn)
		local timer = {
			delay = delay,
			fn = fn,
			killed = false,
			kill = function(self) self.killed = true end,
		}
		state.timers[#state.timers + 1] = timer
		return timer
	end,
	add_key_binding = function(_, name, fn)
		state.binding_names[#state.binding_names + 1] = name
		state["binding_" .. name] = fn
	end,
	register_script_message = function(name, fn)
		state.message_names[#state.message_names + 1] = name
		state["message_" .. name] = fn
	end,
	register_event = function(name, fn)
		state["event_" .. name] = fn
	end,
	observe_property = function() return true end,
	unobserve_property = function() return true end,
	msg = {
		info = function() end,
		warn = function() end,
		error = function() end,
		verbose = function() end,
		debug = function() end,
	},
}

local options_stub = {
	read_options = function(options)
		for key, value in pairs(TEST_OPTIONS) do
			options[key] = value
		end
		return options
	end,
}

local utils_stub = {
	join_path = function(left, right)
		left = tostring(left or "")
		right = tostring(right or "")
		if right:sub(1, 1) == "/" or right:match("^%a:[/\\]") then return right end
		if left == "" then return right end
		if left:sub(-1) == "/" or left:sub(-1) == "\\" then return left .. right end
		return left .. "/" .. right
	end,
	getcwd = function() return CWD end,
	file_info = function(path)
		return FS[path]
	end,
	split_path = function(path)
		local text = tostring(path or ""):gsub("\\", "/")
		local dir = text:match("^(.*)/[^/]*$")
		if dir == nil then return ".", text end
		return dir, text:match("([^/]*)$")
	end,
	format_json = function(value)
		if type(value) ~= "table" then return tostring(value) end
		local parts = {}
		for key, item in pairs(value) do
			local encoded
			if type(item) == "string" then
				encoded = '"' .. item:gsub('\\', '\\\\'):gsub('"', '\\"'):gsub("\n", "\\n") .. '"'
			else
				encoded = tostring(item)
			end
			parts[#parts + 1] = '"' .. tostring(key) .. '":' .. encoded
		end
		return "{" .. table.concat(parts, ",") .. "}"
	end,
	parse_json = function(text)
		if text == nil or text == "" then return nil end
		return nil
	end,
	to_string = function(value) return tostring(value) end,
	readdir = function() return {} end,
	get_env = function() return nil end,
}

package.preload["mp"] = function() return mp_stub end
package.preload["mp.msg"] = function() return mp_stub.msg end
package.preload["mp.options"] = function() return options_stub end
package.preload["mp.utils"] = function() return utils_stub end

-- Loading must never open a file.  dofile() uses the C file API, so replacing
-- the Lua io.open function can safely detect script-level file access.
local real_open = io.open
io.open = function(...)
	error("screenshot_tools.lua accessed a file while loading: " .. tostring((...)))
end
local ok, M = pcall(dofile, script_path)
io.open = real_open
if not ok then
	io.stderr:write("failed to load screenshot_tools.lua: " .. tostring(M) .. "\n")
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

local function count_value(list, value)
	local total = 0
	for _, item in ipairs(list or {}) do
		if item == value then total = total + 1 end
	end
	return total
end

local function sorted(list)
	local copy = {}
	for index, item in ipairs(list or {}) do copy[index] = item end
	table.sort(copy)
	return copy
end

-- ---------------------------------------------------------------------------
-- Load behaviour and public interface
-- ---------------------------------------------------------------------------

ok(type(M) == "table", "script must return a helper table")
if type(M) ~= "table" then os.exit(1) end

eq(#state.subprocess_calls, 0, "no subprocess may run at load time")
eq(#state.osd, 0, "no OSD may be shown at load time")
eq(#state.async_calls, 0, "no async command may run at load time")

local expected_bindings = {
	"burst", "cancel", "capture", "region", "stitch",
	"subtitles", "video", "window",
}
local function same_names(actual, expected, label)
	local left = sorted(actual)
	local right = sorted(expected)
	eq(#left, #right, label .. " count")
	for index = 1, #right do
		eq(left[index], right[index], label .. "[" .. index .. "]")
	end
end
same_names(state.binding_names, expected_bindings, "key bindings")
same_names(state.message_names, expected_bindings, "script messages")
same_names(M.bindings, expected_bindings, "exported binding list")

eq(M.settings.directory, "~~/user/screenshots", "default screenshot directory")
eq(M.settings.queue_file, "~~/user/screenshot-tools/queue.jsonl", "default queue file")
ok(M.settings.queue_file:sub(1, 7) == "~~/user", "queue file lives in the user layer")

-- ---------------------------------------------------------------------------
-- Filenames
-- ---------------------------------------------------------------------------

eq(
	M.build_filename("PlayKit", "png", "subtitles", "20261112-120000", 3),
	"PlayKit-20261112-120000-subtitles-003.png",
	"deterministic filename with burst index"
)
eq(
	M.build_filename("a/b:c*?\"<>|$", "JPEG", "window", "stamp", nil),
	"a-b-c-stamp-window.jpg",
	"prefix sanitization and jpeg alias"
)
eq(
	M.build_filename("..name..", "webp", "video", "stamp", nil),
	"name-stamp-video.webp",
	"leading and trailing dots are removed"
)
eq(
	M.build_filename("", "png", "video", "stamp", nil),
	"screenshot-stamp-video.png",
	"empty prefix falls back to a safe name"
)
ok(M.build_filename("x", "bmp", "video", "stamp", nil) == nil, "bmp must be rejected")
ok(M.build_filename("x", "png", "video", "stamp", 0) == nil, "zero index must be rejected")
ok(M.build_filename("x", "png", "video", "stamp", 1.5) == nil, "fractional index must be rejected")

eq(M.normalize_format("JPEG"), "jpg", "jpeg alias")
eq(M.normalize_format("WebP"), "webp", "case-insensitive webp")
eq(M.normalize_format("bmp"), nil, "unknown format rejected")
eq(M.normalize_mode("WINDOW"), "window", "case-insensitive mode")
eq(M.normalize_direction("vstack"), "vstack", "vstack direction")

-- ---------------------------------------------------------------------------
-- Regions and crops
-- ---------------------------------------------------------------------------

local region = M.parse_region({ "10", "20", "640", "360" })
ok(region ~= nil, "four-argument region must parse")
eq(region.x, 10, "region x")
eq(region.y, 20, "region y")
eq(region.w, 640, "region w")
eq(region.h, 360, "region h")

region = M.parse_region({ "10:20:640:360" })
ok(region ~= nil, "colon region must parse")
eq(region.x, 10, "colon region x")
eq(region.y, 20, "colon region y")
eq(region.w, 640, "colon region w")
eq(region.h, 360, "colon region h")

region = M.parse_region({ "10,20,640,360" })
ok(region ~= nil, "comma region must parse")
eq(region.w, 640, "comma region w")

region = M.parse_region({ "640x360+10+20" })
ok(region ~= nil, "mpv rectangle syntax must parse")
eq(region.x, 10, "rect region x")
eq(region.y, 20, "rect region y")
eq(region.w, 640, "rect region w")
eq(region.h, 360, "rect region h")

ok(M.parse_region({ "10", "20", "640" }) == nil, "three arguments must be rejected")
ok(M.parse_region({ "a", "b", "c", "d" }) == nil, "non-numeric region must be rejected")
ok(M.parse_region({ "10.5", "20", "640", "360" }) == nil, "fractional region must be rejected")
ok(M.parse_region({ "nonsense" }) == nil, "nonsense region must be rejected")

local valid, err = M.validate_region({ x = 0, y = 0, w = 64, h = 48 }, 64, 48)
ok(valid, "full-frame region must be valid: " .. tostring(err))
valid = M.validate_region({ x = 0, y = 0, w = 65, h = 48 }, 64, 48)
ok(not valid, "region wider than the video must be rejected")
valid = M.validate_region({ x = 1, y = 1, w = 64, h = 48 }, 64, 48)
ok(not valid, "region crossing the right/bottom edge must be rejected")
valid = M.validate_region({ x = -1, y = 0, w = 10, h = 10 }, 64, 48)
ok(not valid, "negative origin must be rejected")
valid = M.validate_region({ x = 0, y = 0, w = 0, h = 10 }, 64, 48)
ok(not valid, "zero width must be rejected")
valid = M.validate_region({ x = 0, y = 0, w = 10, h = 10 }, 0, 0)
ok(not valid, "missing video dimensions must be rejected")

eq(M.format_crop({ x = 10, y = 20, w = 640, h = 360 }), "640x360+10+20",
	"video-crop syntax")

-- ---------------------------------------------------------------------------
-- Paths
-- ---------------------------------------------------------------------------

eq(M.normalize_path("C:\\Users\\a\\..\\b"), "C:/Users/b", "windows path normalization")
eq(M.normalize_path("/a/b/../c/./d"), "/a/c/d", "posix path normalization")
eq(M.normalize_path("/"), "/", "root stays root")

ok(M.is_inside("/cfg/user/a.png", "/cfg/user"), "file inside user layer")
ok(M.is_inside("/cfg/user", "/cfg/user/"), "root itself matches with trailing slash")
ok(M.is_inside("/cfg/user/a/../../user/b", "/cfg/user"), "dot-dot stays inside after normalization")
ok(not M.is_inside("/cfg/user2/a.png", "/cfg/user"), "sibling prefix must not match")
ok(not M.is_inside("/cfg/user/../mpv.conf", "/cfg/user"), "dot-dot escape must not match")
ok(not M.is_inside("/cfg/other", "/cfg/user"), "unrelated directory must not match")

-- mpv requires a trailing slash for special "~~" keywords; the script fills it
-- in so a configured "~~desktop" works like "~~desktop/".
eq(M.expand_path("~~desktop"), DESKTOP_ROOT, "bare special keyword is expanded")
eq(M.expand_path("~~desktop/"), DESKTOP_ROOT, "special keyword with slash is expanded")
eq(M.expand_path("~~/user"), CONFIG_ROOT .. "/user", "config-relative path is expanded")
eq(M.expand_path("~~"), CONFIG_ROOT, "config root is expanded")
eq(M.expand_path(""), "", "empty path stays empty")
eq(M.expand_path("relative/shot.png"), "relative/shot.png",
	"ordinary relative paths are left alone")

-- ---------------------------------------------------------------------------
-- Burst planning
-- ---------------------------------------------------------------------------

local times, times_err = M.compute_burst_times(10, 1.5, 3)
ok(times ~= nil, "burst times: " .. tostring(times_err))
eq(#times, 3, "burst time count")
near(times[1], 10, "burst time 1")
near(times[2], 11.5, "burst time 2")
near(times[3], 13, "burst time 3")
ok(M.compute_burst_times(10, 0, 3) == nil, "zero interval must be rejected")
ok(M.compute_burst_times(10, 1, 0) == nil, "zero count must be rejected")
ok(M.compute_burst_times(10, 1, 1001) == nil, "count above the hard cap must be rejected")

local defaults = { count = 5, interval = 1.0, mode = "subtitles" }
local parsed = M.parse_burst_args({ "window" }, defaults)
ok(parsed ~= nil, "mode-only burst arguments")
eq(parsed.mode, "window", "mode-only burst mode")
eq(parsed.count, 5, "mode-only burst count")
eq(parsed.interval, 1.0, "mode-only burst interval")

parsed = M.parse_burst_args({ "3", "0.5", "video" }, defaults)
eq(parsed.count, 3, "burst count argument")
eq(parsed.interval, 0.5, "burst interval argument")
eq(parsed.mode, "video", "burst mode argument")

parsed = M.parse_burst_args({ "3", "window" }, defaults)
eq(parsed.count, 3, "burst count with mode")
eq(parsed.interval, 1.0, "mode keeps the default interval")
eq(parsed.mode, "window", "burst mode without interval")

ok(M.parse_burst_args({ "x" }, defaults) == nil, "non-numeric count must be rejected")
ok(M.parse_burst_args({ "3", "0.5", "video", "extra" }, defaults) == nil,
	"extra burst arguments must be rejected")
ok(M.parse_burst_args({ "3", "-1" }, defaults) == nil, "negative interval must be rejected")

local entries = {
	{ burst = "a", path = "/a1.png" },
	{ burst = "a", path = "/a2.png" },
	{ burst = "b", path = "/b1.png" },
	{ burst = "b", path = "/b2.png" },
}
local files, files_err = M.latest_burst(entries)
ok(files ~= nil, "latest burst: " .. tostring(files_err))
eq(#files, 2, "latest burst file count")
eq(files[1], "/b1.png", "latest burst first file")
eq(files[2], "/b2.png", "latest burst second file")
ok(M.latest_burst({}) == nil, "empty queue must be rejected")
ok(M.latest_burst({ { burst = "a", path = "/a1.png" } }) == nil,
	"single-frame burst must be rejected")
local legacy = M.latest_burst({
	{ path = "/1.png" }, { path = "/2.png" }, { path = "/3.png" },
})
ok(legacy ~= nil and #legacy == 3, "entries without a burst id form one group")

-- ---------------------------------------------------------------------------
-- Image headers and aspect-preserving stitch scaling
-- ---------------------------------------------------------------------------

local function png_bytes(width, height)
	local function u32(value)
		local a = math.floor(value / 16777216) % 256
		local b = math.floor(value / 65536) % 256
		local c = math.floor(value / 256) % 256
		local d = value % 256
		return string.char(a, b, c, d)
	end
	return "\137PNG\r\n\26\n" .. u32(13) .. "IHDR"
		.. u32(width) .. u32(height) .. string.char(8, 2, 0, 0, 0)
end

local function jpeg_bytes(width, height)
	local app0 = "\255\224" .. string.char(0, 16) .. "JFIF\0"
		.. string.char(1, 1, 0, 0, 1, 0, 1, 0, 0)
	local sof = "\255\192" .. string.char(0, 11, 8)
		.. string.char(0, height) .. string.char(0, width)
		.. string.char(1, 1, 17, 0)
	return "\255\216" .. app0 .. sof
end

local function u16le(value)
	return string.char(value % 256, math.floor(value / 256) % 256)
end

local function webp_lossy_bytes(width, height)
	local payload = string.char(0, 0, 0)
		.. "\157\001\042" .. u16le(width) .. u16le(height)
		.. string.char(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
	return "RIFF" .. string.char(0, 0, 0, 0) .. "WEBPVP8 "
		.. string.char(#payload, 0, 0, 0) .. payload
end

local function webp_lossless_bytes(width, height)
	local bits = (width - 1) + (height - 1) * 16384
	local payload = "\047" .. string.char(
		bits % 256,
		math.floor(bits / 256) % 256,
		math.floor(bits / 65536) % 256,
		math.floor(bits / 16777216) % 256
	) .. string.char(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
	return "RIFF" .. string.char(0, 0, 0, 0) .. "WEBPVP8L"
		.. string.char(#payload, 0, 0, 0) .. payload
end

local w, h = M.parse_image_size(png_bytes(640, 360))
eq(w, 640, "png width")
eq(h, 360, "png height")
w, h = M.parse_image_size(jpeg_bytes(200, 100))
eq(w, 200, "jpeg width")
eq(h, 100, "jpeg height")
w, h = M.parse_image_size(webp_lossy_bytes(320, 180))
eq(w, 320, "lossy webp width")
eq(h, 180, "lossy webp height")
w, h = M.parse_image_size(webp_lossless_bytes(480, 270))
eq(w, 480, "lossless webp width")
eq(h, 270, "lossless webp height")
ok(M.parse_image_size("not an image") == nil, "unknown image header must be rejected")
ok(M.parse_image_size("\137PNG\r\n\26\n") == nil, "truncated png must be rejected")

FS["/img/a.png"] = { is_file = true, size = 40 }
FS["/img/b.png"] = { is_file = true, size = 40 }
local real_open2 = io.open
io.open = function(path, mode)
	if mode == "rb" and path == "/img/a.png" then
		local index = 0
		local data = png_bytes(640, 360)
		return {
			read = function()
				index = index + 1
				return index == 1 and data or nil
			end,
			close = function() end,
		}
	end
	if mode == "rb" and path == "/img/b.png" then
		local index = 0
		local data = png_bytes(200, 100)
		return {
			read = function()
				index = index + 1
				return index == 1 and data or nil
			end,
			close = function() end,
		}
	end
	return real_open2(path, mode)
end
local a_size = M.read_image_size("/img/a.png")
local b_size = M.read_image_size("/img/b.png")
io.open = real_open2
ok(a_size ~= nil and a_size.w == 640 and a_size.h == 360,
	"read_image_size probes a png file")
ok(b_size ~= nil and b_size.w == 200 and b_size.h == 100,
	"read_image_size probes the second png")
ok(M.read_image_size("/img/missing.png") == nil,
	"read_image_size returns nil for missing files")

local sizes = { { w = 640, h = 360 }, { w = 200, h = 100 } }
local specs = M.stitch_scale_specs(sizes, "hstack")
ok(specs ~= nil, "mixed-height inputs need hstack scaling")
eq(specs[1], nil, "the tallest image is left unchanged")
eq(specs[2], "scale=-2:360", "hstack scales the short image by height")
specs = M.stitch_scale_specs(sizes, "vstack")
ok(specs ~= nil, "mixed-width inputs need vstack scaling")
eq(specs[1], nil, "the widest image is left unchanged")
eq(specs[2], "scale=640:-2", "vstack scales the narrow image by width")
ok(M.stitch_scale_specs({ { w = 640, h = 360 }, { w = 640, h = 360 } }, "hstack") == nil,
	"equal heights do not need scaling")
ok(M.stitch_scale_specs({ { w = 640 }, { w = 640, h = 360 } }, "hstack") == nil,
	"incomplete dimensions do not need scaling")

local mixed_job = {
	inputs = { "/a.png", "/b.png" },
	output = "/out.png",
	direction = "hstack",
	format = "png",
	quality = { png_compression = 4 },
	sizes = sizes,
}
local mixed_mpv_args = M.build_mpv_stitch_args({
	mpv = "/pkg/mpv",
	inputs = mixed_job.inputs,
	output = mixed_job.output,
	direction = mixed_job.direction,
	format = mixed_job.format,
	quality = mixed_job.quality,
	sizes = mixed_job.sizes,
})
contains(mixed_mpv_args,
	"--lavfi-complex=[vid2]scale=-2:360[sv2];[vid1][sv2]hstack=inputs=2[vo]",
	"mpv graph scales only the second image")
local mixed_ffmpeg_args = M.build_ffmpeg_stitch_args({
	ffmpeg = "ffmpeg",
	inputs = mixed_job.inputs,
	output = mixed_job.output,
	direction = mixed_job.direction,
	format = mixed_job.format,
	quality = mixed_job.quality,
	sizes = mixed_job.sizes,
})
contains(mixed_ffmpeg_args,
	"[1:v]scale=-2:360[sv1];[0:v][sv1]hstack=inputs=2[v]",
	"ffmpeg graph scales only the second image")

-- ---------------------------------------------------------------------------
-- Stitch argument generation
-- ---------------------------------------------------------------------------

local quality = {
	jpeg_quality = 100,
	png_compression = 4,
	webp_lossless = true,
	webp_quality = 80,
	webp_compression = 5,
}
eq(M.jpeg_qscale(100), 2, "jpeg quality 100 maps to qscale 2")
eq(M.jpeg_qscale(0), 31, "jpeg quality 0 maps to qscale 31")
eq(M.jpeg_qscale(50), 16, "jpeg quality 50 maps to qscale 16")
eq(M.mpv_quality_args("png", quality)[1], "--ovcopts=compression_level=4",
	"mpv png quality argument")
eq(M.mpv_quality_args("jpg", quality)[1], "--ovcopts=qscale=2",
	"mpv jpg quality argument")
eq(M.mpv_quality_args("webp", quality)[1], "--ovcopts=lossless=1",
	"mpv webp lossless argument")
eq(M.mpv_quality_args("webp", {
	webp_lossless = false, webp_quality = 80, webp_compression = 5,
})[1], "--ovcopts=quality=80,compression_level=5",
	"mpv webp quality argument")

local args, args_err = M.build_mpv_stitch_args({
	mpv = "/pkg/mpv",
	inputs = { "/a.png", "/b.png", "/c.png" },
	output = "/out.png",
	direction = "hstack",
	format = "png",
	quality = quality,
})
ok(args ~= nil, "mpv stitch args: " .. tostring(args_err))
contains(args, "--no-config", "mpv args")
contains(args, "--frames=1", "mpv args")
contains(args, "--of=image2", "mpv args")
contains(args, "--o=/out.png", "mpv args")
contains(args, "--ovc=png", "mpv args")
contains(args, "--ofopts=update=1", "mpv args")
contains(args, "--ovcopts=compression_level=4", "mpv args")
contains(args, "--lavfi-complex=[vid1][vid2][vid3]hstack=inputs=3[vo]",
	"mpv lavfi graph")
contains(args, "--external-file=/b.png", "mpv external input")
contains(args, "--external-file=/c.png", "mpv external input")
eq(args[#args], "/a.png", "first input is the positional file")
eq(count_value(args, "--external-file=/b.png"), 1, "external file appears once")

args = M.build_mpv_stitch_args({
	mpv = "/pkg/mpv",
	inputs = { "/a.png", "/b.png" },
	output = "/out.png",
	direction = "vstack",
	format = "png",
	quality = quality,
})
ok(args ~= nil, "mpv vstack args")
contains(args, "--lavfi-complex=[vid1][vid2]vstack=inputs=2[vo]",
	"mpv vstack graph")

args = M.build_mpv_stitch_args({
	mpv = "/pkg/mpv",
	inputs = { "/a.png", "/b.png" },
	output = "/out.jpg",
	direction = "hstack",
	format = "jpg",
	quality = quality,
})
ok(args ~= nil, "mpv jpg args")
contains(args, "--ovc=mjpeg", "mpv jpeg codec")
contains(args, "--ovcopts=qscale=2", "mpv jpeg quality")

args = M.build_mpv_stitch_args({
	mpv = "/pkg/mpv",
	inputs = { "/a.png", "/b.png" },
	output = "/out.webp",
	direction = "hstack",
	format = "webp",
	quality = quality,
})
ok(args ~= nil, "mpv webp args")
contains(args, "--ovc=libwebp", "mpv webp codec")
contains(args, "--ovcopts=lossless=1", "mpv webp lossless")

ok(M.build_mpv_stitch_args({
	mpv = "/pkg/mpv",
	inputs = { "/a.png" },
	output = "/out.png",
	direction = "hstack",
	format = "png",
}) == nil, "mpv stitch needs two inputs")
ok(M.build_mpv_stitch_args({
	mpv = "/pkg/mpv",
	inputs = { "/a.png", "/b.png" },
	output = "/out.png",
	direction = "diagonal",
	format = "png",
}) == nil, "mpv stitch rejects an unknown direction")
ok(M.build_mpv_stitch_args({
	mpv = "/pkg/mpv",
	inputs = { "/a.png", "/b.png" },
	output = "/out.png",
	direction = "hstack",
	format = "bmp",
}) == nil, "mpv stitch rejects an unknown format")

args, args_err = M.build_ffmpeg_stitch_args({
	ffmpeg = "ffmpeg",
	inputs = { "/a.png", "/b.png" },
	output = "/out.jpg",
	direction = "vstack",
	format = "jpg",
	quality = quality,
})
ok(args ~= nil, "ffmpeg stitch args: " .. tostring(args_err))
contains(args, "-hide_banner", "ffmpeg args")
contains(args, "-nostdin", "ffmpeg args")
contains(args, "-filter_complex", "ffmpeg args")
contains(args, "[0:v][1:v]vstack=inputs=2[v]", "ffmpeg filter graph")
contains(args, "-map", "ffmpeg args")
contains(args, "[v]", "ffmpeg output map")
contains(args, "-frames:v", "ffmpeg args")
contains(args, "-c:v", "ffmpeg args")
contains(args, "mjpeg", "ffmpeg jpeg codec")
contains(args, "-q:v", "ffmpeg jpeg quality")
contains(args, "2", "ffmpeg jpeg qscale")
contains(args, "-update", "ffmpeg image update")
eq(count_value(args, "-i"), 2, "ffmpeg input count")
eq(args[#args], "/out.jpg", "ffmpeg output is last")

args = M.build_ffmpeg_stitch_args({
	ffmpeg = "ffmpeg",
	inputs = { "/a.png", "/b.png" },
	output = "/out.png",
	direction = "hstack",
	format = "png",
	quality = quality,
})
ok(args ~= nil, "ffmpeg png args")
contains(args, "-compression_level", "ffmpeg png compression")
contains(args, "4", "ffmpeg png compression value")

args = M.build_ffmpeg_stitch_args({
	ffmpeg = "ffmpeg",
	inputs = { "/a.png", "/b.png" },
	output = "/out.webp",
	direction = "hstack",
	format = "webp",
	quality = quality,
})
ok(args ~= nil, "ffmpeg webp args")
contains(args, "libwebp", "ffmpeg webp codec")
contains(args, "-lossless", "ffmpeg webp lossless")
contains(args, "1", "ffmpeg webp lossless value")

args = M.build_ffmpeg_stitch_args({
	ffmpeg = "ffmpeg",
	inputs = { "/a.png", "/b.png" },
	output = "/out.webp",
	direction = "hstack",
	format = "webp",
	quality = { webp_lossless = false, webp_quality = 80, webp_compression = 5 },
})
ok(args ~= nil, "ffmpeg lossy webp args")
contains(args, "-quality", "ffmpeg webp quality")
contains(args, "80", "ffmpeg webp quality value")
contains(args, "0", "ffmpeg webp lossless disabled")

ok(M.build_ffmpeg_stitch_args({
	ffmpeg = "ffmpeg",
	inputs = {},
	output = "/out.png",
	direction = "hstack",
	format = "png",
}) == nil, "ffmpeg stitch needs two inputs")

if failures > 0 then
	io.stderr:write(string.format("%d test(s) failed\n", failures))
	os.exit(1)
end

print("screenshot_tools Lua tests passed")
