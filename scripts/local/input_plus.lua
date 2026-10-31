-- input_plus.lua - small, original input helpers for mpv_PlayKit
--
-- The historical mpv_PlayKit input_plus.lua is not used here.  This file
-- implements the two features that the portable configuration exposes:
-- a key-name inspector and the documentation link menu.  It is intentionally
-- small and has no external dependencies.

local mp = require "mp"
local msg = require "mp.msg"
local mp_options = require "mp.options"

local options = {
	load = true,
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
local inspector_active = false

local function open_url(url)
	if type(url) ~= "string" or not url:match("^https://") then
		mp.osd_message("没有可用的文档地址", 3000)
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

local HELP_URLS = {
	[0] = "https://github.com/100pangci/mpv_PlayKit_TLS",
	[1] = "https://mpv.io/manual/master/",
	[2] = "https://hooke007.github.io/official_man/mpv.html",
	[3] = "https://github.com/mpv-player/mpv/wiki/FAQ",
}

local function lazy_helper(index)
	local number = tonumber(index)
	local url = number and HELP_URLS[number]
	if not url then
		mp.osd_message("未知的帮助条目", 3000)
		return
	end
	open_url(url)
end

local function describe_binding(key)
	local bindings = mp.get_property_native("input-bindings") or {}
	local commands = {}
	for _, binding in ipairs(bindings) do
		if binding.key == key and binding.cmd and binding.cmd ~= "" then
			commands[#commands + 1] = binding.cmd
			if #commands >= 4 then break end
		end
	end
	if #commands == 0 then return "未绑定" end
	return table.concat(commands, "\n")
end

local function show_key(key)
	local text = string.format("按键：%s\n%s\n\n再次按 Ctrl+Alt+i 退出按键检测", key, describe_binding(key))
	mp.osd_message(text, 4)
end

local function stop_inspector()
	for _, name in ipairs({
		"input_plus-inspector",
		"input_plus-inspector-unmapped",
		"input_plus-inspector-exit",
	}) do
		pcall(mp.remove_key_binding, name)
	end
	inspector_active = false
	mp.osd_message("按键检测已关闭", 2000)
end

local function start_inspector()
	inspector_active = true
	mp.add_forced_key_binding("any_unicode", "input_plus-inspector", function(key)
		show_key(tostring(key))
	end)
	mp.add_forced_key_binding("unmapped", "input_plus-inspector-unmapped", function(key)
		show_key(tostring(key))
	end)
	mp.add_forced_key_binding("Ctrl+Alt+i", "input_plus-inspector-exit", stop_inspector)
	mp.osd_message("按键检测已开启：按任意键查看名称与绑定，Ctrl+Alt+i 退出", 4000)
end

local function toggle_inspector()
	if inspector_active then
		stop_inspector()
	else
		start_inspector()
	end
end

mp.add_key_binding(nil, "input_test_toggle", toggle_inspector)
mp.register_script_message("lazy-helper", lazy_helper)
mp.register_script_message("stop-inspector", stop_inspector)
