-- thumbfast.lua - clean-room thumbnail engine for mpv_PlayKit
--
-- uosc 5.13 asks a script named "thumbfast" for timeline thumbnails.  This
-- implementation is written for this project and does not contain code from
-- the historical mpv_PlayKit thumb_engine fork.  It decodes one frame with a
-- short-lived mpv subprocess and displays the raw BGRA frame with mpv's
-- overlay-add command.  Frames are cached per file and per time bucket.

local mp = require "mp"
local msg = require "mp.msg"
local utils = require "mp.utils"
local mp_options = require "mp.options"

local options = {
	enabled = true,
	mpv_path = "",
	max_width = 270,
	max_height = 270,
	min_duration = 30,
	precise = 5,
	network = true,
	cache_dir = "~~/_cache/thumbfast",
	timeout_seconds = 15,
	hwdec = "no",
}
mp_options.read_options(options, "thumbfast")

local function disabled(value)
	return value == false or value == 0 or value == "no" or value == "false" or value == "0"
end

if disabled(options.enabled) then
	msg.info("thumbfast is disabled by configuration")
	return
end

local platform = mp.get_property("platform") or "linux"
local overlay_id = 42
local state = {
	loaded = false,
	enabled_by_user = true,
	path = nil,
	duration = 0,
	width = 0,
	height = 0,
	generating = false,
	request = nil,
	visible = false,
	failures = 0,
}

local function expand(path)
	return mp.command_native({ "expand-path", path })
end

local function absolute(path)
	if path:match("^%a:[/\\]") or path:sub(1, 1) == "/" then return path end
	return utils.join_path(utils.getcwd(), path)
end

local function file_exists(path)
	local info = utils.file_info(path)
	return info and info.is_file
end

local function ensure_dir(path)
	local info = utils.file_info(path)
	if info and info.is_dir then return true end
	local command
	if platform == "windows" then
		command = 'mkdir "' .. path:gsub("/", "\\") .. '" >NUL 2>NUL'
	else
		command = 'mkdir -p "' .. path .. '" >/dev/null 2>&1'
	end
	os.execute(command)
	info = utils.file_info(path)
	if info and info.is_dir then return true end
	msg.warn("could not create cache directory " .. path)
	return false
end

local function auto_mpv_path()
	local candidates = {}
	if platform == "windows" then
		candidates = { "~~/../mpv.exe", "mpv.exe" }
	else
		candidates = { "~~/../mpv/bin/mpv", "mpv" }
	end
	for _, candidate in ipairs(candidates) do
		if candidate:find("~~", 1, true) then
			local expanded = expand(candidate)
			if file_exists(expanded) then return absolute(expanded) end
		else
			return candidate
		end
	end
	return candidates[#candidates]
end

local mpv_path = options.mpv_path ~= "" and options.mpv_path or auto_mpv_path()
local cache_root = absolute(expand(options.cache_dir))

local CLIENT_SCRIPT = [==[
local mp = require "mp"
local utils = require "mp.utils"
local out = mp.get_opt("out")
local target = tonumber(mp.get_opt("target") or "0")
local tries = 0

local function capture()
	tries = tries + 1
	local image = mp.command_native({ "screenshot-raw", "video" })
	if image and image.data and #image.data > 0 then
		local file = io.open(out, "wb")
		if not file then mp.command("quit") return end
		file:write(image.data)
		file:close()
		local meta = io.open(out .. ".json", "w")
		if meta then
			meta:write(utils.format_json({ w = image.w, h = image.h, stride = image.stride }))
			meta:close()
		end
		mp.command("quit")
	elseif tries < 50 then
		mp.add_timeout(0.1, capture)
	else
		mp.command("quit")
	end
end

mp.register_event("file-loaded", function()
	mp.set_property("pause", "yes")
	if target > 0 then mp.commandv("seek", target, "absolute", "exact") end
	mp.add_timeout(0.4, capture)
end)
]==]

local client_script = cache_root .. "/thumbfast-client.lua"

local function write_client()
	if file_exists(client_script) then return true end
	local file = io.open(client_script, "wb")
	if not file then return false end
	file:write(CLIENT_SCRIPT)
	file:close()
	return true
end

local function cache_key()
	local path = state.path or ""
	local size = mp.get_property_number("file-size", 0) or 0
	local raw = string.format("%s|%.0f|%.3f", path, size, state.duration)
	local hash = 5381
	for index = 1, #raw do
		hash = (hash * 33 + raw:byte(index)) % 4294967296
	end
	return string.format("%08x", hash)
end

local function send_info(width, height)
	local payload = utils.format_json({ width = width, height = height })
	mp.commandv("script-message-to", "uosc", "thumbfast-info", payload)
end

local function clear_overlay()
	if state.visible then
		pcall(mp.command_native, { "overlay-remove", overlay_id })
		state.visible = false
	end
end

local function display(request, width, height, stride)
	if not state.loaded or not state.enabled_by_user or not state.path then return end
	local ok, err = pcall(mp.command_native, {
		"overlay-add",
		overlay_id,
		request.x,
		request.y,
		request.base .. ".bgra",
		0,
		"bgra",
		width,
		height,
		stride,
		width,
		height,
	})
	if not ok then
		msg.warn("overlay-add failed: " .. tostring(err))
		return
	end
	state.visible = true
	if width ~= state.width or height ~= state.height then
		state.width, state.height = width, height
		send_info(width, height)
	end
end

local function read_meta(path)
	local file = io.open(path, "rb")
	if not file then return nil end
	local contents = file:read("*a")
	file:close()
	if not contents or contents == "" then return nil end
	local ok, data = pcall(utils.parse_json, contents)
	if not ok or type(data) ~= "table" then return nil end
	local width = tonumber(data.w)
	local height = tonumber(data.h)
	local stride = tonumber(data.stride)
	if not width or not height or not stride or width < 1 or height < 1 then return nil end
	return width, height, stride
end

local function request_next()

	if state.generating or not state.request then return end
	local request = state.request
	state.request = nil

	local width, height, stride = read_meta(request.base .. ".bgra.json")
	if width then
		display(request, width, height, stride)
		if state.request then request_next() end
		return
	end

	if not ensure_dir(cache_root) or not write_client() then
		msg.warn("thumbnail cache is unavailable")
		return
	end

	state.generating = true
	local args = {
		mpv_path,
		"--no-config",
		"--no-terminal",
		"--no-audio",
		"--no-osc",
		"--no-input-default-bindings",
		"--vo=null",
		"--hwdec=" .. tostring(options.hwdec),
		"--pause",
		"--keep-open=yes",
		"--start=" .. string.format("%.3f", request.time),
		"--vf=scale=w=" .. tostring(options.max_width) .. ":h=" .. tostring(options.max_height)
			.. ":force_original_aspect_ratio=decrease",
		"--script=" .. client_script,
		"--script-opts=out=" .. request.base .. ".bgra,target=0",
		state.path,
	}

	local completed = false
	local function finish(ok)
		if completed then return end
		completed = true
		state.generating = false
		if not ok then
			state.failures = state.failures + 1
			msg.warn("thumbnail subprocess failed for " .. tostring(state.path))
			if state.failures >= 3 then
				msg.warn("disabling thumbnails after repeated failures")
				state.loaded = false
				send_info(0, 0)
			end
		else
			state.failures = 0
		end
		if state.request then request_next() end
	end

	mp.add_timeout(math.max(3, tonumber(options.timeout_seconds) or 15), function()
		if state.generating and not completed then
			msg.warn("thumbnail generation timed out")
			finish(false)
		end
	end)

	mp.command_native_async({
		name = "subprocess",
		args = args,
		playback_only = false,
		capture_stdout = true,
		capture_stderr = true,
	}, function(success, result, error)
		if not success or not result or result.status ~= 0 then
			msg.warn("thumbnail subprocess result: " .. tostring(utils.to_string(result)) .. " error=" .. tostring(error))
			finish(false)
			return
		end
		local w, h, s = read_meta(request.base .. ".bgra.json")
		if not w then
			finish(false)
			return
		end
		if state.loaded then
			display(request, w, h, s)
		end
		finish(true)
	end)
end

local function on_file_loaded()
	clear_overlay()
	state.loaded = false
	state.path = mp.get_property("path")
	state.duration = mp.get_property_number("duration", 0) or 0
	state.width, state.height = 0, 0
	state.request = nil
	state.failures = 0

	if not state.enabled_by_user or not state.path or state.path == "" then
		send_info(0, 0)
		return
	end
	if state.duration < math.max(0, tonumber(options.min_duration) or 0) then
		send_info(0, 0)
		return
	end
	local is_network = state.path:match("^%a[%w+.-]*://") ~= nil
	if is_network and disabled(options.network) then
		send_info(0, 0)
		return
	end

	local aspect = mp.get_property_number("video-params/aspect", 0) or 0
	if aspect <= 0 then
		local width = mp.get_property_number("width", 0) or 0
		local height = mp.get_property_number("height", 0) or 0
		if width > 0 and height > 0 then aspect = width / height end
	end
	if aspect <= 0 then
		send_info(0, 0)
		return
	end

	local max_width = math.max(16, tonumber(options.max_width) or 270)
	local max_height = math.max(16, tonumber(options.max_height) or 270)
	local width, height
	if aspect >= 1 then
		width = max_width
		height = math.floor(max_width / aspect + 0.5)
	else
		height = max_height
		width = math.floor(max_height * aspect + 0.5)
	end
	width = math.max(2, width - width % 2)
	height = math.max(2, height - height % 2)
	state.width, state.height = width, height
	state.loaded = true
	send_info(width, height)
end

local function on_thumb(time, x, y)
	if not state.loaded or not state.enabled_by_user or not state.path then return end
	local target = tonumber(time)
	local pos_x = tonumber(x)
	local pos_y = tonumber(y)
	if not target or not pos_x or not pos_y then return end
	if target < 0 then target = 0 end
	if state.duration > 0 and target > state.duration then target = state.duration end
	local step = math.max(1, tonumber(options.precise) or 5)
	local bucket = math.floor(target / step) * step
	local key = string.format("%08d", math.floor(bucket))
	state.request = {
		time = bucket,
		x = math.floor(pos_x),
		y = math.floor(pos_y),
		base = cache_root .. "/" .. cache_key() .. "-" .. key,
	}
	request_next()
end

mp.register_script_message("thumb", on_thumb)
mp.register_script_message("uosc-version", function()
	if state.loaded then send_info(state.width, state.height) end
end)
mp.register_script_message("clear", function()
	state.request = nil
	clear_overlay()
end)
mp.register_event("file-loaded", on_file_loaded)
mp.register_event("end-file", function()
	state.loaded = false
	state.request = nil
	clear_overlay()
end)
mp.register_event("shutdown", function()
	clear_overlay()
end)

mp.add_key_binding(nil, "rerun", function()
	clear_overlay()
	state.failures = 0
	if state.loaded then send_info(state.width, state.height) end
end)

mp.add_key_binding(nil, "toggle", function()
	state.enabled_by_user = not state.enabled_by_user
	state.request = nil
	clear_overlay()
	if state.enabled_by_user then
		if state.loaded then send_info(state.width, state.height) end
		mp.osd_message("缩略图预览已开启", 2000)
	else
		send_info(0, 0)
		mp.osd_message("缩略图预览已关闭", 2000)
	end
end)
