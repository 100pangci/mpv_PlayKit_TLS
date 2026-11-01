-- input_plus.lua - original input helpers for mpv PlayKit
--
-- This is a clean-room implementation for this project.  It provides the
-- features referenced by config/common/input.conf and menu.conf, grouped by
-- priority:
--
--   P0  key-name inspector, documentation links
--   P1  information panel, file/URL/ISO import, audio-device cycling,
--       chapter tools, wall clock
--   P2  command/stat cycling, track cycling, dB volume, quit confirmation,
--       playlist tools, speed/sync helpers, AB audio mixing
--
-- The script uses only mpv's Lua API and its own OSD overlays.  It never
-- invokes PowerShell or any other external interpreter.

local mp = require "mp"
local msg = require "mp.msg"
local utils = require "mp.utils"
local mp_options = require "mp.options"
local input_api = nil
do
	local ok, module = pcall(require, "mp.input")
	if ok and type(module) == "table" then
		input_api = module
	end
end

local options = {
	load = true,
	osd_duration = 2,
	info_refresh = 2,
	chapter_keywords = "OP$,opening$,ED$,ending$,オープニング$,エンディング$",
	playlist_temp = "~~/playlist_temp.mpl",
	speed_target = 3,
	speed_iterations = 10,
	speed_delta_max = 0.5,
	speed_delta_min = 0.0005,
}

mp_options.read_options(options, "input_plus")

local function disabled(value)
	return value == false or value == 0 or value == "no" or value == "false" or value == "0"
end

if disabled(options.load) then
	msg.info("input_plus is disabled by configuration")
	return
end

local platform = mp.get_property("platform") or "linux"
local script_name = mp.get_script_name()

math.randomseed(os.time())

local function osd(text, duration)
	mp.osd_message("[input_plus] " .. text, duration or options.osd_duration)
end

local function escape_ass(text)
	text = tostring(text or "")
	text = text:gsub("\\", "\\\\")
	text = text:gsub("{", "\\{")
	text = text:gsub("}", "\\}")
	return text
end

local function format_time(seconds)
	seconds = tonumber(seconds)
	if not seconds or seconds < 0 then return "--:--" end
	seconds = math.floor(seconds)
	local hours = math.floor(seconds / 3600)
	local minutes = math.floor(seconds % 3600 / 60)
	local rest = seconds % 60
	if hours > 0 then
		return string.format("%d:%02d:%02d", hours, minutes, rest)
	end
	return string.format("%02d:%02d", minutes, rest)
end

local function expand(path)
	return mp.command_native({ "expand-path", path })
end

local function open_url(url)
	if type(url) ~= "string" or not url:match("^https://") then
		osd("没有可用的文档地址")
		return
	end
	local args
	if platform == "windows" then
		args = { "explorer.exe", url }
	else
		args = { "xdg-open", url }
	end
	mp.command_native_async({
		name = "subprocess",
		args = args,
		playback_only = false,
	}, function() end)
end

--
-- P0: key-name inspector
--

local KEY_SECTION = "input_plus-key-inspector"
local key_overlay = mp.create_osd_overlay("ass-events")
key_overlay.res_x = 1280
key_overlay.res_y = 720

local inspector = {
	active = false,
	hint = "",
	last_key = nil,
	hide_timer = nil,
	forced = {},
}

local IGNORED_KEYS = {
	CLOSE_WIN = true,
	MOUSE_ENTER = true,
	MOUSE_LEAVE = true,
	MOUSE_MOVE = true,
}

local NAV_KEYS = {
	UP = true, DOWN = true, LEFT = true, RIGHT = true,
	HOME = true, END = true, PGUP = true, PGDWN = true,
}

local SPECIAL_KEYS = {
	ESC = true, TAB = true, ENTER = true, BS = true, DEL = true,
	INS = true, SPACE = true, PRINT = true, IDEOGRAPHIC_SPACE = true,
}

local REPLAY_EVENT = {
	down = "keydown",
	up = "keyup",
	press = "keypress",
	["repeat"] = "keypress",
}

local function key_color(name)
	if name:match("^F%d+$") or name:match("^KP") then return "&H7A2E80&" end
	if NAV_KEYS[name] then return "&H2E8B00&" end
	if name:match("^MBTN") or name:match("^MOUSE_BTN")
		or name:match("^WHEEL_") or name:match("^AXIS_") then
		return "&H998800&"
	end
	if name:match("^GAMEPAD_") or name:match("^TABLET_") then return "&H2E6B8B&" end
	if name:match("^XF86_") then return "&H6B238E&" end
	if SPECIAL_KEYS[name] then return "&H0048B8&" end
	if name:match("^%u[%u_]+$") and #name > 2 then return "&H6B238E&" end
	return "&H333333&"
end

local function split_combo(name)
	local modifiers, base = name:match("^(.*%+)([^+]+)$")
	if modifiers and (modifiers:find("Ctrl%+") or modifiers:find("Alt%+")
		or modifiers:find("Shift%+") or modifiers:find("Meta%+")) then
		return modifiers, base
	end
	return nil, name
end

local function rounded_rect(width, height, radius)
	local c = 0.5519 * radius
	return table.concat({
		string.format("m %d 0", radius),
		string.format("l %d 0", width - radius),
		string.format("b %d 0 %d %d %d %d", width - radius + c, width, radius - c, width, radius),
		string.format("l %d %d", width, height - radius),
		string.format("b %d %d %d %d %d %d", width, height - radius + c, width - radius + c, height, width - radius, height),
		string.format("l %d %d", radius, height),
		string.format("b %d %d 0 %d 0 %d", radius - c, height, height - radius + c, height - radius),
		string.format("l 0 %d", radius),
		string.format("b 0 %d %d 0 %d 0", radius - c, radius - c, radius),
	}, " ")
end

local function describe_binding(key)
	local commands = {}
	for _, binding in ipairs(mp.get_property_native("input-bindings", {}) or {}) do
		if binding.key == key and binding.cmd and binding.cmd ~= "" then
			commands[#commands + 1] = binding.cmd
			if #commands >= 3 then break end
		end
	end
	if #commands == 0 then return "未绑定" end
	return table.concat(commands, "\n")
end

local function inspector_show(name)
	if inspector.hide_timer then
		inspector.hide_timer:kill()
		inspector.hide_timer = nil
	end

	local modifiers, base = split_combo(name)
	local text
	if modifiers then
		text = "{\\1c&H9E4A00&}" .. escape_ass(modifiers)
			.. "{\\1c" .. key_color(base) .. "}" .. escape_ass(base)
	else
		text = "{\\1c" .. key_color(name) .. "}" .. escape_ass(name)
	end

	local font_size = 64
	local text_width = math.max(#name * font_size * 0.55, font_size)
	local pad_x, pad_y = 36, 18
	local width = text_width + pad_x * 2
	local height = font_size + pad_y * 2
	local radius = 16
	local cx, cy = key_overlay.res_x / 2, key_overlay.res_y / 2

	local background = string.format(
		"{\\rDefault\\an5\\pos(%d,%d)\\p1\\1c&HE8E8E8&\\1a&H18&\\bord0\\shad0}%s",
		cx, cy, rounded_rect(width, height, radius)
	)
	local description = escape_ass(describe_binding(name)):gsub("\n", "\\N")
	local foreground = string.format(
		"{\\rDefault\\an5\\pos(%d,%d)\\fs%d\\b1\\bord0\\shad0}%s",
		cx, cy, font_size, text
	)
	local detail = string.format(
		"{\\rDefault\\an7\\pos(48,110)\\fs22\\bord1\\shad0\\1c&HFFFFFF&\\3c&H000000&}绑定：%s",
		description
	)

	key_overlay.data = inspector.hint .. "\n" .. background .. "\n" .. foreground .. "\n" .. detail
	key_overlay:update()
	inspector.last_key = name

	inspector.hide_timer = mp.add_timeout(2, function()
		key_overlay.data = inspector.hint
		key_overlay:update()
		inspector.hide_timer = nil
	end)
end

local function toggle_keys()
	local keys, seen = {}, {}
	for _, binding in ipairs(mp.get_property_native("input-bindings", {}) or {}) do
		if binding.key and binding.cmd and binding.cmd:find("input_test_toggle", 1, true)
			and not seen[binding.key] then
			seen[binding.key] = true
			keys[#keys + 1] = binding.key
		end
	end
	return keys
end

local function inspector_section()
	local lines = {
		"any_unicode script-binding " .. script_name .. "/input-inspector-handler",
		"unmapped script-binding " .. script_name .. "/input-inspector-handler",
		"ESC script-binding " .. script_name .. "/input-inspector-handler",
	}
	for _, key in ipairs(toggle_keys()) do
		lines[#lines + 1] = key .. " script-binding " .. script_name .. "/input_test_toggle"
	end
	return table.concat(lines, "\n")
end

local function remove_forced_bindings()
	for _, name in ipairs(inspector.forced) do
		pcall(mp.remove_key_binding, name)
	end
	inspector.forced = {}
end

local function inspector_deactivate()
	inspector.active = false
	if type(mp.input_disable_section) == "function" then
		pcall(mp.input_disable_section, KEY_SECTION)
		pcall(mp.input_define_section, KEY_SECTION, "")
	end
	remove_forced_bindings()
	if inspector.hide_timer then
		inspector.hide_timer:kill()
		inspector.hide_timer = nil
	end
	key_overlay:remove()
end

local function replay_key(name, event)
	local command = REPLAY_EVENT[event]
	if not command then return end
	if type(mp.input_disable_section) == "function" then
		pcall(mp.input_disable_section, KEY_SECTION)
	end
	pcall(mp.commandv, command, name)
	if type(mp.input_enable_section) == "function" then
		mp.add_timeout(0, function()
			if inspector.active then
				pcall(mp.input_enable_section, KEY_SECTION, "exclusive")
			end
		end)
	end
end

local function inspector_on_key(info)
	if not inspector.active then return end
	local name = info.key_name or info.key_text or "?"
	if name == "ESC" and (info.event == "down" or info.event == "press") then
		inspector_deactivate()
		osd("按键检测已关闭")
		return
	end
	if IGNORED_KEYS[name] then
		replay_key(name, info.event)
		return
	end
	if info.event == "down" or info.event == "press" then
		inspector_show(name)
	end
end

local function inspector_activate()
	local toggle = toggle_keys()
	inspector.hint = "{\\rDefault\\an1\\fs40\\bord2\\shad0\\1c&HFFFFFF&\\3c&H000000&}按键检测模式"
		.. "（" .. table.concat(#toggle > 0 and toggle or { "ESC" }, " / ") .. " / ESC 退出）"

	local handler_options = { complex = true, repeatable = true }

	if type(mp.input_define_section) == "function"
		and type(mp.input_enable_section) == "function" then
		pcall(mp.input_define_section, KEY_SECTION, inspector_section(), "force")
		pcall(mp.input_enable_section, KEY_SECTION, "exclusive")
	else
		local function add(name, key, callback, opts)
			inspector.forced[#inspector.forced + 1] = name
			mp.add_forced_key_binding(key, name, callback, opts or handler_options)
		end
		add("input-inspector-any", "any_unicode", inspector_on_key)
		add("input-inspector-unmapped", "unmapped", inspector_on_key)
		add("input-inspector-esc", "ESC", function()
			inspector_deactivate()
			osd("按键检测已关闭")
		end)
		for index, key in ipairs(toggle) do
			add("input-inspector-toggle-" .. index, key, function()
				if inspector.active then
					inspector_deactivate()
					osd("按键检测已关闭")
				end
			end)
		end
	end

	inspector.active = true
	key_overlay.data = inspector.hint
	key_overlay:update()
end

local function inspector_toggle()
	if inspector.active then
		inspector_deactivate()
		osd("按键检测已关闭")
	else
		inspector_activate()
		osd("按键检测已开启：按任意键查看名称与绑定，ESC 退出")
	end
end

--
-- P0: documentation links
--

local HELP_URLS = {
	[0] = "https://github.com/100pangci/mpv_PlayKit_TLS",
	[1] = "https://mpv.io/manual/master/",
	[2] = "https://hooke007.github.io/official_man/mpv.html",
	[3] = "https://github.com/mpv-player/mpv/wiki/FAQ",
	[5] = "https://mpv.io/manual/master/#options-glsl-shaders",
}

local function lazy_helper(index)
	local number = tonumber(index)
	if number == 4 then
		osd("本发行版不包含 VapourSynth 滤镜；内置滤镜可在视频菜单中切换")
		return
	end
	local url = number and HELP_URLS[number]
	if not url then
		osd("未知的帮助条目")
		return
	end
	open_url(url)
end

--
-- P1: information panel
--

local info_overlay = mp.create_osd_overlay("ass-events")
info_overlay.res_x = 1280
info_overlay.res_y = 720
local info_visible = false
local info_timer = nil

local function info_value(value)
	if value == nil or value == "" then return "-" end
	value = tostring(value):gsub("\r", " "):gsub("\n", " ")
	return escape_ass(value)
end

local function info_text()
	local video = mp.get_property_native("video-params") or {}
	local audio = mp.get_property_native("audio-params") or {}
	local tracks = mp.get_property_native("track-list") or {}
	local counts = { video = 0, audio = 0, sub = 0 }
	for _, track in ipairs(tracks) do
		if counts[track.type] ~= nil then
			counts[track.type] = counts[track.type] + 1
		end
	end

	local position = mp.get_property_number("time-pos", 0) or 0
	local duration = mp.get_property_number("duration", 0) or 0
	local width = video.w or mp.get_property_number("width", 0) or 0
	local height = video.h or mp.get_property_number("height", 0) or 0
	local fps = mp.get_property_number("container-fps", 0) or 0
	local fps_target = mp.get_property_number("estimated-vf-fps", 0) or 0
	local video_bitrate = (mp.get_property_number("video-bitrate", 0) or 0) / 1000
	local audio_bitrate = (mp.get_property_number("audio-bitrate", 0) or 0) / 1000
	local size = mp.get_property_number("file-size", 0) or 0
	local path = mp.get_property("path", "") or ""
	local config_dir = mp.get_property_bool("config") and (expand("~~/") or "-") or "no"
	local dimensions = mp.get_property_native("osd-dimensions") or {}
	local output_width = (dimensions.w or 0) - (dimensions.ml or 0) - (dimensions.mr or 0)
	local output_height = (dimensions.h or 0) - (dimensions.mt or 0) - (dimensions.mb or 0)

	local lines = {
		"{\\fs26\\b1}mpv PlayKit 信息面板",
		"文件： " .. info_value(mp.get_property("media-title") or mp.get_property("filename")),
		"路径： " .. info_value(path),
		"设置目录： " .. info_value(config_dir),
		"输出尺寸： " .. output_width .. "x" .. output_height,
		"时长： " .. format_time(position) .. " / " .. format_time(duration),
		"容器： " .. info_value(mp.get_property("file-format")),
		"大小： " .. string.format("%.1f MiB", size / 1024 / 1024),
		"",
		"视频： " .. info_value(mp.get_property("video-codec")) .. "   " .. width .. "x" .. height,
		"帧率： " .. string.format("%.3f", fps) .. " / " .. string.format("%.3f", fps_target)
			.. "  像素： " .. info_value(video.pixelformat or video["hw-pixelformat"]),
		"输出： " .. info_value(mp.get_property("current-vo")) .. "  硬解： " .. info_value(mp.get_property("hwdec-current")),
		"码率： " .. string.format("%.0f kbps", video_bitrate) .. "  丢帧： " .. (mp.get_property_number("frame-drop-count", 0) or 0),
		"同步： " .. info_value(mp.get_property("video-sync")),
		"",
		"音频： " .. info_value(mp.get_property("audio-codec")) .. "  " .. info_value(audio.channels) .. " 声道",
		"采样： " .. info_value(audio.samplerate) .. " Hz  码率： " .. string.format("%.0f kbps", audio_bitrate),
		"",
		"轨道： 视频 " .. counts.video .. " / 音频 " .. counts.audio .. " / 字幕 " .. counts.sub,
		"章节： " .. (mp.get_property_number("chapters", 0) or 0),
		"循环： " .. info_value(mp.get_property("loop-file")) .. " / " .. info_value(mp.get_property("loop-playlist")),
		"",
		"视频滤镜： " .. info_value(mp.get_property("vf")),
		"着色器： " .. info_value(mp.get_property("glsl-shaders")),
		"音频滤镜： " .. info_value(mp.get_property("af")),
	}

	return "{\\an7\\pos(28,24)\\fs18\\bord1\\shad0\\1c&HFFFFFF&\\3c&H000000&}"
		.. table.concat(lines, "\\N")
end

local function info_refresh()
	if not info_visible then return end
	info_overlay.data = info_text()
	info_overlay:update()
end

local function info_hide()
	info_visible = false
	if info_timer then
		info_timer:kill()
		info_timer = nil
	end
	info_overlay:remove()
end

local function info_toggle()
	if info_visible then
		info_hide()
		osd("信息面板已关闭")
		return
	end
	info_visible = true
	info_refresh()
	info_timer = mp.add_periodic_timer(
		math.max(1, tonumber(options.info_refresh) or 2),
		info_refresh
	)
	osd("信息面板已开启")
end

--
-- P1: import helpers (no external dialog implementation)
--

local function prompt_text(prompt, default, callback)
	if input_api and type(input_api.get) == "function" then
		input_api.get({
			prompt = prompt,
			default = default or "",
			submit = callback,
		})
		return true
	end
	osd("此功能需要 mpv 0.39 或更新的输入 API")
	return false
end

local function load_file(path, mode)
	if type(path) ~= "string" or path:gsub("%s", "") == "" then return end
	mode = mode or (mp.get_property_native("idle-active") and "replace" or "append-play")
	mp.commandv("loadfile", path, mode)
end

local function uosc_available()
	local scripts = tostring(mp.get_property("scripts", "") or ""):lower()
	return scripts:find("uosc", 1, true) ~= nil
end

local function import_files()
	if uosc_available() then
		mp.commandv("script-binding", "uosc/open-file")
	else
		prompt_text("输入文件路径", "", function(path) load_file(path, "replace") end)
	end
end

local function import_url()
	prompt_text("输入 URL", "https://", function(url)
		load_file(url, "replace")
	end)
end

local function import_iso(kind)
	prompt_text(kind == 1 and "输入 DVD ISO 路径" or "输入蓝光 ISO 路径", "", function(path)
		if kind == 1 then
			mp.commandv("set", "dvd-device", path)
			mp.commandv("loadfile", "dvd://", "replace")
		else
			mp.commandv("set", "bluray-device", path)
			mp.commandv("loadfile", "bd://", "replace")
		end
	end)
end

local function import_external(kind)
	prompt_text(kind == "audio" and "输入外部音轨路径" or "输入外部字幕路径", "", function(path)
		if kind == "audio" then
			mp.commandv("audio-add", path, "auto")
		else
			mp.commandv("sub-add", path, "cached")
		end
	end)
end

--
-- P1: audio-device cycling
--

local function audio_devices(all)
	local devices, result = mp.get_property_native("audio-device-list", {}) or {}, {}
	local current_ao = mp.get_property("current-ao", "") or ""
	for _, device in ipairs(devices) do
		if all or current_ao == "" or device.name:find(current_ao, 1, true) then
			result[#result + 1] = device
		end
	end
	return result
end

local function cycle_audio_device(all, delta)
	local devices = audio_devices(all)
	if #devices == 0 then
		osd("没有可用的音频输出设备")
		return
	end
	local current = mp.get_property("audio-device", "")
	local index = 1
	for position, device in ipairs(devices) do
		if device.name == current then
			index = position
			break
		end
	end
	index = (index - 1 + delta) % #devices + 1
	local device = devices[index]
	mp.set_property("audio-device", device.name)
	osd("音频输出：" .. tostring(device.description or device.name))
end

--
-- P1: chapter tools
--

local skipping_chapters = false
local chapter_skip_enabled = false

local function chapter_keywords()
	local result = {}
	for keyword in tostring(options.chapter_keywords or ""):gmatch("[^,]+") do
		result[#result + 1] = keyword:gsub("^%s+", ""):gsub("%s+$", "")
	end
	return result
end

local function chapter_title_matches(title)
	for _, pattern in ipairs(chapter_keywords()) do
		if pattern ~= "" and title:match(pattern) then return true end
	end
	return false
end

local function chapter_skip_check(_, value)
	if not chapter_skip_enabled or not value or skipping_chapters then return end
	if chapter_title_matches(tostring(value)) then
		skipping_chapters = true
		mp.commandv("add", "chapter", 1)
		mp.add_timeout(0.2, function() skipping_chapters = false end)
	end
end

local function chapter_skip_toggle()
	chapter_skip_enabled = not chapter_skip_enabled
	if chapter_skip_enabled then
		mp.observe_property("chapter-metadata/TITLE", "string", chapter_skip_check)
		osd("已启用跳过片头片尾")
	else
		mp.unobserve_property(chapter_skip_check)
		osd("已禁用跳过片头片尾")
	end
end

local function chapter_step(step, natural, use_uosc)
	step = tonumber(step) or 0
	if step == 0 then return end
	local direction = step > 0 and 1 or -1
	local path = mp.get_property("path", "")
	if path == "" then return end

	local chapters = mp.get_property_native("chapter-list", {}) or {}
	local count = #chapters
	if count == 0 then
		osd("没有章节")
		return
	end
	local current = mp.get_property_number("chapter", 0) or 0
	local target = current + direction

	if target < 0 or target >= count then
		local ok = false
		if use_uosc then
			ok = mp.commandv("script-binding", "uosc/" .. (direction > 0 and "next" or "prev"))
		end
		if not ok then
			mp.commandv("playlist-" .. (direction > 0 and "next" or "prev"), "weak")
		end
		if natural and direction < 0 then
			mp.add_timeout(0.25, function()
				if mp.get_property("path", "") ~= path then
					local count_now = #(mp.get_property_native("chapter-list", {}) or {})
					if count_now > 0 then
						mp.set_property_number("chapter", count_now - 1)
					end
				end
			end)
		end
		return
	end
	mp.commandv("add", "chapter", direction)
end

--
-- P1: wall clock
--

local clock_overlay = mp.create_osd_overlay("ass-events")
clock_overlay.res_x = 1280
clock_overlay.res_y = 720
local clock_visible = false
local clock_timer = nil

local function clock_draw()
	local now = os.date("*t")
	clock_overlay.data = "{\\an9\\pos(1250,24)\\fs30\\bord2\\shad0\\1c&H01DBF1&\\3c&H000000&}"
		.. string.format("%02d:%02d:%02d", now.hour, now.min, now.sec)
	clock_overlay:update()
end

local function clock_hide()
	clock_visible = false
	if clock_timer then
		clock_timer:kill()
		clock_timer = nil
	end
	clock_overlay:remove()
end

local function clock_toggle()
	if clock_visible then
		clock_hide()
		return
	end
	clock_visible = true
	clock_draw()
	clock_timer = mp.add_periodic_timer(1, clock_draw)
end

local function clock_display()
	if clock_visible then return end
	clock_draw()
	mp.add_timeout(2, function()
		if not clock_visible then clock_overlay:remove() end
	end)
end

--
-- P2: command and statistics cycling
--

local cycle_positions = {}

local function cycle_cmds(...)
	local commands = { ... }
	if #commands == 0 then return end
	local key = table.concat(commands, "|")
	cycle_positions[key] = (cycle_positions[key] or 0) % #commands + 1
	local command = commands[cycle_positions[key]]
	if command and command ~= "" then mp.command(command) end
end

local stats_page = 0

local function cycle_stats(first, last)
	first = math.max(1, math.min(5, tonumber(first) or 1))
	last = math.max(first, math.min(5, tonumber(last) or 5))
	stats_page = stats_page % last + 1
	if stats_page < first then stats_page = first end
	mp.commandv("script-binding", "display-page-" .. stats_page)
end

--
-- P2: track cycling and dB volume
--

local function track_step(prop, delta)
	mp.commandv("add", prop, delta)
	if mp.get_property_number(prop, 0) == 0 then
		mp.commandv("add", prop, delta)
		if mp.get_property_number(prop, 0) == 0 then
			osd("没有可用的" .. prop .. "轨道")
		end
	end
end

local function track_refresh(prop)
	local current = mp.get_property_number(prop, 0)
	if current == 0 then
		osd("当前 " .. prop .. " 无效")
		return
	end
	mp.set_property_number(prop, 0)
	mp.set_property_number(prop, current)
end

local function swap_secondary_subtitles()
	local main = mp.get_property_number("sid", 0)
	local secondary = mp.get_property_number("secondary-sid", 0)
	if main == 0 and secondary == 0 then return end
	mp.set_property_number("sid", 0)
	mp.set_property_number("secondary-sid", 0)
	mp.set_property_number("sid", secondary)
	mp.set_property_number("secondary-sid", main)
end

local function volume_to_db(volume)
	return 60.0 * math.log(volume / 100.0) / math.log(10.0)
end

local function db_to_volume(db)
	return math.exp(math.log(10.0) * (db / 60.0 + 2))
end

local function volume_add_db(delta)
	local volume = mp.get_property_number("volume", 100) or 100
	if volume <= 1 then
		osd("音量过低，无法以 dB 调整")
		return
	end
	local gain = math.floor(volume_to_db(volume) + 0.5) + delta
	local maximum = mp.get_property_number("volume-max", 130) or 130
	if db_to_volume(gain) > maximum then gain = volume_to_db(maximum) end
	if db_to_volume(gain) < 10 then gain = volume_to_db(10) end
	mp.set_property_number("volume", db_to_volume(gain))
	osd(string.format("音量增益： %+.2f dB", gain))
end

--
-- P2: quit confirmation
--

local quit_pending = false
local quit_timer = nil
local quit_double = false
local quit_double_timer = nil

local function quit_wait()
	if quit_pending then
		quit_pending = false
		if quit_timer then
			quit_timer:kill()
			quit_timer = nil
		end
		osd("已取消退出")
		return
	end
	quit_pending = true
	osd("即将退出，再次执行以取消", 3)
	quit_timer = mp.add_timeout(3, function()
		quit_timer = nil
		if quit_pending then
			quit_pending = false
			mp.command("quit")
		end
	end)
end

local function quit_real()
	if quit_double then
		if quit_double_timer then
			quit_double_timer:kill()
			quit_double_timer = nil
		end
		quit_double = false
		mp.command("quit")
		return
	end
	quit_double = true
	osd("再次执行以确认退出", 1.5)
	quit_double_timer = mp.add_timeout(1.5, function()
		quit_double = false
		quit_double_timer = nil
	end)
end

--
-- P2: playlist tools
--

local playlist_shuffled = false
local playlist_busy = false

local function show_playlist()
	mp.add_timeout(0.1, function()
		local text = mp.command_native({ "expand-text", "${playlist}" }) or ""
		mp.osd_message(text, 2)
	end)
end

local function playlist_order(mode, replay)
	if playlist_busy then return end
	if (mp.get_property_number("playlist-count", 0) or 0) <= 2 then
		osd("播放列表条目不足")
		return
	end
	playlist_busy = true
	if mode == 1 and not playlist_shuffled then
		mp.command("playlist-unshuffle")
		mp.command("playlist-shuffle")
		playlist_shuffled = true
	elseif mode == 1 then
		mp.command("playlist-shuffle")
		playlist_shuffled = true
	elseif playlist_shuffled then
		mp.command("playlist-unshuffle")
		playlist_shuffled = false
	else
		mp.command("playlist-shuffle")
		playlist_shuffled = true
	end
	if replay then mp.commandv("playlist-play-index", 0) end
	show_playlist()
	mp.add_timeout(0.2, function() playlist_busy = false end)
end

local function playlist_random()
	local count = mp.get_property_number("playlist-count", 0) or 0
	if count <= 2 then
		osd("播放列表条目数量未超过 2")
		return
	end
	local current = mp.get_property_number("playlist-pos-1", 0) or 0
	local target = math.random(1, count)
	while target == current do target = math.random(1, count) end
	mp.set_property_number("playlist-pos-1", target)
end

local function playlist_temp_path()
	local root = expand(options.playlist_temp) or options.playlist_temp
	return root
end

local function playlist_tmp_save()
	local count = mp.get_property_number("playlist-count", 0) or 0
	if count == 0 then
		osd("播放列表中没有文件")
		return
	end
	local path = playlist_temp_path()
	local file, error_message = io.open(path, "w")
	if not file then
		osd("播放列表保存失败：" .. tostring(error_message))
		return
	end
	file:write("#EXTM3U\n")
	for index = 0, count - 1 do
		local filename = mp.get_property("playlist/" .. index .. "/filename")
		if filename and filename ~= "" then file:write(filename, "\n") end
	end
	file:close()
	osd("已保存临时播放列表")
end

local function playlist_tmp_load()
	local path = playlist_temp_path()
	local info = utils.file_info(path)
	if not info or not info.is_file then
		osd("临时播放列表不存在")
		return
	end
	mp.commandv("loadlist", path, "replace")
	mp.add_timeout(0.2, function()
		if (mp.get_property_number("playlist-count", 0) or 0) == 0 then
			osd("临时播放列表加载失败")
		else
			show_playlist()
		end
	end)
end

--
-- P2: speed helpers
--

local previous_speed = nil

local function speed_recover()
	local speed = mp.get_property_number("speed", 1) or 1
	if math.abs(speed - 1) > 0.0001 then
		previous_speed = speed
		mp.set_property_number("speed", 1)
		osd("已恢复常速")
	else
		local restore = previous_speed or 1
		mp.set_property_number("speed", restore)
		osd(string.format("已恢复播放速度 %.2f", restore))
	end
end

local function speed_candidate(ratio, factor)
	local scaled = ratio * factor
	local rounded = math.floor(scaled + 0.5)
	if rounded < 1 then return nil end
	local candidate = scaled / rounded
	local delta = math.abs(candidate - 1)
	if delta < (tonumber(options.speed_delta_min) or 0.0005) then return 1 end
	if delta <= (tonumber(options.speed_delta_max) or 0.5) then return candidate end
	return nil
end

local speed_adaptive_enabled = false

local function speed_adaptive()
	local raw = mp.get_property_number("container-fps", 0) or 0
	local actual = mp.get_property_number("estimated-vf-fps", 0) or 0
	local display = mp.get_property_number("display-fps", 0) or 0
	local current = mp.get_property_number("speed", 1) or 1
	if raw == 0 or display == 0 or raw > 32 or math.abs(actual - raw) > 0.5 then
		msg.warn("speed_sync_toggle: unusual frame rate, leaving speed unchanged")
		return
	end
	local iterations = math.max(1, math.floor(tonumber(options.speed_iterations) or 10))
	for index = 1, iterations do
		local candidate = speed_candidate(display / raw, index)
		if candidate then
			if math.abs(candidate - current) >= 0.0001 then
				mp.set_property_number("speed", candidate)
				msg.info(string.format("speed_sync_toggle: speed set to %.6f", candidate))
			end
			return
		end
	end
	msg.warn("speed_sync_toggle: no suitable speed found")
end

local function speed_sync_toggle()
	speed_adaptive_enabled = not speed_adaptive_enabled
	if speed_adaptive_enabled then
		speed_adaptive()
		mp.register_event("playback-restart", speed_adaptive)
		osd("已启用速度自适应")
	else
		mp.unregister_event(speed_adaptive)
		mp.set_property_number("speed", 1)
		osd("已禁用速度自适应")
	end
end

local function speed_hold(rate)
	return function(event)
		if event.event == "down" then
			mp.set_property_number("speed", rate)
		elseif event.event == "up" then
			mp.set_property_number("speed", 1)
		end
	end
end

local function speed_auto_target()
	return speed_hold(tonumber(options.speed_target) or 3)
end

--
-- P2: hold-to-seek acceleration
--

local seek_acc_duration = 0
local seek_acc_step = 1

local function seek_acc(back, exact)
	return function(event)
		if event.event == "up" then
			seek_acc_duration = 0
			return
		end
		if event.event ~= "repeat" then return end
		seek_acc_duration = seek_acc_duration + seek_acc_step
		local distance = (back and "-" or "") .. seek_acc_duration
		mp.command("seek " .. distance .. " " .. (exact and "exact" or "keyframes"))
	end
end

--
-- P2: AB audio mixing
--

local marked_a = nil
local marked_b = nil
local merged = false
local mark_registered = false

local function mark_reset()
	mp.commandv("set", "lavfi-complex", "")
	merged = false
	marked_a, marked_b = nil, nil
	if mark_registered then
		mp.unregister_event(mark_reset)
		mark_registered = false
	end
end

local function mark_a()
	marked_a = mp.get_property_number("aid", 0)
	if marked_a == 0 then
		marked_a = nil
		osd("当前音轨无效")
	else
		osd("已标记当前音轨为 A：" .. marked_a)
	end
	if not mark_registered then
		mp.register_event("end-file", mark_reset)
		mark_registered = true
	end
end

local function mark_b()
	marked_b = mp.get_property_number("aid", 0)
	if marked_b == 0 then
		marked_b = nil
		osd("当前音轨无效")
	else
		osd("已标记当前音轨为 B：" .. marked_b)
	end
	if not mark_registered then
		mp.register_event("end-file", mark_reset)
		mark_registered = true
	end
end

local function mark_merge()
	if marked_a == nil or marked_b == nil or marked_a == marked_b then
		osd("无效的 AB 音轨")
		marked_a, marked_b = nil, nil
		return
	end
	mp.commandv("set", "lavfi-complex",
		"[aid" .. marked_a .. "] [aid" .. marked_b .. "] amix [ao]")
	merged = true
	osd("已合并 A/B 音轨")
end

local function mark_finish()
	if merged then
		mark_reset()
		mp.commandv("set", "aid", "auto")
		osd("已取消音轨并轨")
		return
	end
	if marked_a == nil then
		mark_a()
	elseif marked_b == nil then
		mark_b()
	else
		mark_merge()
	end
end

--
-- Hold-to-bypass and secondary subtitle helpers
--

local function hold_property(prop)
	local saved = nil
	return function(event)
		if event.event == "down" then
			saved = mp.get_property_native(prop)
			mp.set_property_native(prop, "")
			msg.verbose("cleared " .. prop)
		elseif event.event == "up" then
			mp.set_property_native(prop, saved)
			saved = nil
			msg.verbose("restored " .. prop)
		end
	end
end

--
-- Registration
--

mp.add_key_binding(nil, "input_test_toggle", inspector_toggle)
mp.add_key_binding(nil, "input-inspector-handler", inspector_on_key, {
	complex = true,
	repeatable = true,
})
mp.register_script_message("lazy-helper", lazy_helper)
mp.register_script_message("stop-inspector", function()
	if inspector.active then
		inspector_deactivate()
		osd("按键检测已关闭")
	end
end)

mp.add_key_binding(nil, "info_toggle", info_toggle)
mp.add_key_binding(nil, "import_files", import_files)
mp.add_key_binding(nil, "import_iso_dvd", function() import_iso(1) end)
mp.add_key_binding(nil, "import_iso_bd", function() import_iso(2) end)
mp.add_key_binding(nil, "import_url", import_url)
mp.add_key_binding(nil, "import_append_aid", function() import_external("audio") end)
mp.add_key_binding(nil, "import_append_sid", function() import_external("subtitle") end)

mp.add_key_binding(nil, "adevice_back", function() cycle_audio_device(false, -1) end)
mp.add_key_binding(nil, "adevice_next", function() cycle_audio_device(false, 1) end)
mp.add_key_binding(nil, "adevice_all_back", function() cycle_audio_device(true, -1) end)
mp.add_key_binding(nil, "adevice_all_next", function() cycle_audio_device(true, 1) end)

mp.add_key_binding(nil, "chap_skip_toggle", chapter_skip_toggle)
mp.add_key_binding(nil, "chapter_back", function() chapter_step(-1, false, true) end)
mp.add_key_binding(nil, "chapter_back_nat", function() chapter_step(-1, true, false) end)
mp.add_key_binding(nil, "chapter_next", function() chapter_step(1, false, true) end)
mp.register_script_message("chapter-useek", function(direction, natural)
	chapter_step(tonumber(direction) or 0, natural == true or natural == "yes", true)
end)

mp.add_key_binding(nil, "ostime_toggle", clock_toggle)
mp.add_key_binding(nil, "ostime_display", clock_display)

mp.register_script_message("cycle-cmds", cycle_cmds)
mp.register_script_message("cycle-stats", function(first, last)
	cycle_stats(tonumber(first), tonumber(last))
end)

mp.add_key_binding(nil, "trackA_back", function() track_step("aid", -1) end)
mp.add_key_binding(nil, "trackA_next", function() track_step("aid", 1) end)
mp.add_key_binding(nil, "trackS_back", function() track_step("sid", -1) end)
mp.add_key_binding(nil, "trackS_next", function() track_step("sid", 1) end)
mp.add_key_binding(nil, "trackV_back", function() track_step("vid", -1) end)
mp.add_key_binding(nil, "trackV_next", function() track_step("vid", 1) end)
mp.add_key_binding(nil, "trackA_refresh", function() track_refresh("aid") end)
mp.add_key_binding(nil, "trackS_refresh", function() track_refresh("sid") end)
mp.add_key_binding(nil, "trackV_refresh", function() track_refresh("vid") end)
mp.add_key_binding(nil, "sids_sec_swap", swap_secondary_subtitles)

mp.add_key_binding(nil, "volume_db_dec", function() volume_add_db(-1) end, { repeatable = true })
mp.add_key_binding(nil, "volume_db_inc", function() volume_add_db(1) end, { repeatable = true })

mp.add_key_binding(nil, "quit_wait", quit_wait)
mp.add_key_binding(nil, "quit_real", quit_real)

mp.add_key_binding(nil, "playlist_order_0", function() playlist_order(0, false) end)
mp.add_key_binding(nil, "playlist_order_0r", function() playlist_order(0, true) end)
mp.add_key_binding(nil, "playlist_order_1", function() playlist_order(1, false) end)
mp.add_key_binding(nil, "playlist_order_1r", function() playlist_order(1, true) end)
mp.add_key_binding(nil, "playlist_random", playlist_random)
mp.add_key_binding(nil, "playlist_tmp_save", playlist_tmp_save)
mp.add_key_binding(nil, "playlist_tmp_load", playlist_tmp_load)

mp.add_key_binding(nil, "speed_recover", speed_recover)
mp.add_key_binding(nil, "speed_sync_toggle", speed_sync_toggle)
mp.add_key_binding(nil, "speed_auto", speed_hold(2), { complex = true })
mp.add_key_binding(nil, "speed_auto_bullet", speed_hold(0.5), { complex = true })
mp.add_key_binding(nil, "speed_autox", speed_auto_target(), { complex = true })

mp.add_key_binding(nil, "seek_acc", seek_acc(false, false), { complex = true })
mp.add_key_binding(nil, "seek_acc_back", seek_acc(true, false), { complex = true })
mp.add_key_binding(nil, "seek_acc_alt", seek_acc(false, true), { complex = true })
mp.add_key_binding(nil, "seek_acc_back_alt", seek_acc(true, true), { complex = true })

mp.add_key_binding(nil, "mark_aid_A", mark_a)
mp.add_key_binding(nil, "mark_aid_B", mark_b)
mp.add_key_binding(nil, "mark_aid_merge", mark_merge)
mp.add_key_binding(nil, "mark_aid_reset", function()
	mark_reset()
	osd("已取消音轨标记与并轨")
end)
mp.add_key_binding(nil, "mark_aid_fin", mark_finish)

mp.add_key_binding(nil, "af_hold", hold_property("af"), { complex = true })
mp.add_key_binding(nil, "vf_hold", hold_property("vf"), { complex = true })
mp.add_key_binding(nil, "glsl_hold", hold_property("glsl-shaders"), { complex = true })

mp.register_event("file-loaded", function()
	if info_visible then info_refresh() end
end)

mp.register_event("shutdown", function()
	info_hide()
	clock_hide()
	if inspector.active then inspector_deactivate() end
end)