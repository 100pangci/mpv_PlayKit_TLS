-- screenshot_tools.lua - original screenshot utilities for mpv PlayKit
--
-- This script is newly authored for mpv_PlayKit.  It provides still capture in
-- video/subtitles/window modes, temporary rectangular crops, burst capture and
-- image stitching.  Stitching prefers the bundled mpv binary and falls back to
-- ffmpeg from PATH only when necessary.  Every subprocess receives a structured
-- argument list; no shell command is assembled from strings.
--
-- Loading the script only reads options and registers bindings.  Directories,
-- screenshots and queue records are created on demand, never at load time.

local mp = require "mp"
local msg = require "mp.msg"
local utils = require "mp.utils"
local mp_options = require "mp.options"

local settings = {
	enabled = true,
	directory = "~~/user/screenshots",
	queue_file = "~~/user/screenshot-tools/queue.jsonl",
	filename_prefix = "PlayKit",
	format = "png",
	default_mode = "subtitles",
	region = "",
	capture_delay = 0.2,
	file_timeout = 10,
	burst_count = 5,
	burst_interval = 1.0,
	burst_mode = "subtitles",
	burst_max_count = 200,
	stitch_direction = "hstack",
	mpv_path = "",
	ffmpeg_path = "ffmpeg",
	subprocess_timeout = 120,
}
mp_options.read_options(settings, "screenshot_tools")

local platform = mp.get_property("platform") or "linux"

local MODES = { video = true, subtitles = true, window = true }
local FORMATS = { png = true, jpg = true, webp = true }
local DIRECTIONS = { hstack = true, vstack = true }

-- ---------------------------------------------------------------------------
-- Pure helpers.  These are also exercised by tests/lua/screenshot_tools.lua.
-- ---------------------------------------------------------------------------

local function trim(value)
	return (tostring(value or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end

local function number_or(value, fallback)
	local number = tonumber(value)
	if number == nil then return fallback end
	return number
end

local function disabled(value)
	return value == false or value == 0 or value == "no" or value == "false" or value == "0"
end

local function parse_integer(value)
	local text = trim(value)
	if not text:match("^%-?%d+$") then return nil end
	local number = tonumber(text)
	if number == nil or number ~= math.floor(number) then return nil end
	return number
end

local function normalize_format(value)
	local text = trim(value):lower()
	if text == "jpeg" then text = "jpg" end
	if FORMATS[text] then return text end
	return nil
end

local function normalize_mode(value)
	local text = trim(value):lower()
	if MODES[text] then return text end
	return nil
end

local function normalize_direction(value)
	local text = trim(value):lower()
	if DIRECTIONS[text] then return text end
	return nil
end

-- Produce a filename-safe token.  Windows forbids a larger set of characters
-- than POSIX, so sanitize for the strictest platform on both.  The dollar sign
-- is removed because mpv expands ${...} in command parameters.
local function sanitize_prefix(value)
	local text = tostring(value or "")
	text = text:gsub("%c", "-")
	text = text:gsub('[/\\:*?"<>|$]', "-")
	text = text:gsub("%-+", "-")
	text = text:gsub("^[%s%.%-]+", "")
	text = text:gsub("[%s%.%-]+$", "")
	if text == "" then text = "screenshot" end
	if #text > 80 then text = text:sub(1, 80) end
	return text
end

local function build_filename(prefix, format, label, stamp, index)
	local extension = normalize_format(format)
	if not extension then return nil, "截图格式必须是 png、jpg 或 webp" end
	local stamp_text = tostring(stamp or "")
	if stamp_text == "" then stamp_text = os.date("%Y%m%d-%H%M%S") end
	local parts = {
		sanitize_prefix(prefix),
		sanitize_prefix(stamp_text),
		sanitize_prefix(label),
	}
	if index ~= nil then
		local number = tonumber(index)
		if number == nil or number < 1 or number ~= math.floor(number) then
			return nil, "截图序号必须是正整数"
		end
		parts[#parts + 1] = string.format("%03d", number)
	end
	return table.concat(parts, "-") .. "." .. extension
end

-- Accept "x y w h" (four arguments), "x:y:w:h", "x,y,w,h" or mpv's
-- "WxH+X+Y" rectangle spelling.  Parsing and validation are deliberately
-- separate: parse_region() may return a negative rectangle so that
-- validate_region() can report the exact reason to the user.
local function parse_region(args)
	if type(args) ~= "table" then
		return nil, "截图区域参数无效"
	end
	local x, y, w, h
	if #args == 4 then
		x, y, w, h = parse_integer(args[1]), parse_integer(args[2]),
			parse_integer(args[3]), parse_integer(args[4])
		if x == nil or y == nil or w == nil or h == nil then
			return nil, "截图区域必须是整数 x y w h"
		end
	elseif #args == 1 then
		local text = trim(args[1])
		local sx, sy, sw, sh = text:match(
			"^%s*(%-?%d+)%s*[:%,]%s*(%-?%d+)%s*[:%,]%s*(%-?%d+)%s*[:%,]%s*(%-?%d+)%s*$"
		)
		if sx then
			x, y, w, h = tonumber(sx), tonumber(sy), tonumber(sw), tonumber(sh)
		else
			local rw, rh, rx, ry = text:match(
				"^%s*(%-?%d+)%s*[xX]%s*(%-?%d+)%s*%+%s*(%-?%d+)%s*%+%s*(%-?%d+)%s*$"
			)
			if rw then
				w, h, x, y = tonumber(rw), tonumber(rh), tonumber(rx), tonumber(ry)
			end
		end
		if x == nil then
			return nil, "无法解析截图区域，请使用 x:y:w:h 或 WxH+X+Y"
		end
	else
		return nil, "截图区域需要 4 个整数或 x:y:w:h"
	end
	return { x = x, y = y, w = w, h = h }
end

local function validate_region(region, width, height)
	if type(region) ~= "table" then
		return false, "截图区域无效"
	end
	local x, y, w, h = region.x, region.y, region.w, region.h
	for _, value in ipairs({ x, y, w, h }) do
		if type(value) ~= "number" or value ~= math.floor(value) then
			return false, "截图区域必须是整数"
		end
	end
	if x < 0 or y < 0 or w <= 0 or h <= 0 then
		return false, "截图区域起点不能为负，宽高必须大于 0"
	end
	if type(width) ~= "number" or type(height) ~= "number" or width <= 0 or height <= 0 then
		return false, "当前视频尺寸无效"
	end
	if x + w > width or y + h > height then
		return false, string.format("截图区域超出视频范围（视频 %dx%d）", width, height)
	end
	return true
end

local function format_crop(region)
	return string.format("%dx%d+%d+%d", region.w, region.h, region.x, region.y)
end

-- Normalize a filesystem path for containment checks without touching disk.
local function normalize_path(path)
	local text = tostring(path or ""):gsub("\\", "/")
	local drive = text:match("^(%a:)")
	local rest = text
	if drive then rest = text:sub(3) end
	local absolute = rest:sub(1, 1) == "/"
	local parts = {}
	for part in rest:gmatch("[^/]+") do
		if part == ".." then
			if #parts > 0 and parts[#parts] ~= ".." then
				table.remove(parts)
			elseif not absolute then
				parts[#parts + 1] = ".."
			end
		elseif part ~= "." then
			parts[#parts + 1] = part
		end
	end
	local result = table.concat(parts, "/")
	if drive then
		if result == "" then return drive .. "/" end
		return drive .. "/" .. result
	end
	if absolute then return "/" .. result end
	return result
end

local function is_inside(path, root)
	local candidate = normalize_path(path)
	local base = normalize_path(root)
	if base == "" then return false end
	base = base:gsub("/+$", "")
	if base == "" then base = "/" end
	if platform == "windows" then
		candidate = candidate:lower()
		base = base:lower()
	end
	if candidate == base then return true end
	if base == "/" then return candidate:sub(1, 1) == "/" end
	return candidate:sub(1, #base + 1) == base .. "/"
end

local function clamp_int(value, low, high, fallback)
	local number = tonumber(value)
	if number == nil then number = fallback end
	number = math.floor(number + 0.5)
	if number < low then number = low end
	if number > high then number = high end
	return number
end

-- Map mpv's 0..100 JPEG quality to FFmpeg's 2..31 qscale (lower is better).
local function jpeg_qscale(quality)
	local value = math.max(0, math.min(100, number_or(quality, 100)))
	local scale = 31 - math.floor((value * 29 + 50) / 100)
	if scale < 2 then return 2 end
	if scale > 31 then return 31 end
	return scale
end

local function compute_burst_times(start, interval, count)
	start = tonumber(start)
	interval = tonumber(interval)
	count = tonumber(count)
	if start == nil then return nil, "播放位置无效" end
	if interval == nil or interval <= 0 or interval > 3600 then
		return nil, "连拍间隔必须大于 0 且不超过 3600 秒"
	end
	if count == nil or count ~= math.floor(count) or count < 1 or count > 1000 then
		return nil, "连拍数量必须在 1 到 1000 之间"
	end
	local times = {}
	for index = 1, count do
		times[index] = start + (index - 1) * interval
	end
	return times
end

-- Parse "burst [mode] [count] [interval] [mode]" arguments.
local function parse_burst_args(args, defaults)
	defaults = defaults or {}
	local result = {
		count = tonumber(defaults.count) or 5,
		interval = tonumber(defaults.interval) or 1.0,
		mode = normalize_mode(defaults.mode) or "subtitles",
	}
	local index = 1
	if type(args) == "table" and args[index] ~= nil then
		local mode = normalize_mode(args[index])
		if mode then
			result.mode = mode
			index = index + 1
		end
	end
	if type(args) == "table" and args[index] ~= nil then
		local count = parse_integer(args[index])
		if count == nil then return nil, "连拍数量必须是整数" end
		result.count = count
		index = index + 1
	end
	if type(args) == "table" and args[index] ~= nil then
		local interval = tonumber(args[index])
		if interval ~= nil then
			result.interval = interval
			index = index + 1
		else
			local mode = normalize_mode(args[index])
			if mode == nil then return nil, "连拍间隔必须是数字" end
			result.mode = mode
			index = index + 1
		end
	end
	if type(args) == "table" and args[index] ~= nil then
		local mode = normalize_mode(args[index])
		if mode == nil then
			return nil, "连拍模式必须是 video、subtitles 或 window"
		end
		result.mode = mode
		index = index + 1
	end
	if type(args) == "table" and args[index] ~= nil then
		return nil, "连拍参数过多"
	end
	if result.count < 1 then return nil, "连拍数量必须大于 0" end
	if result.interval <= 0 or result.interval > 3600 then
		return nil, "连拍间隔必须大于 0 且不超过 3600 秒"
	end
	return result
end

-- Return the trailing contiguous group of queue entries that belongs to the
-- most recent burst.  Entries without a burst id are treated as one group.
local function latest_burst(entries)
	if type(entries) ~= "table" or #entries == 0 then
		return nil, "没有可用的连拍记录"
	end
	local last = entries[#entries]
	if type(last) ~= "table" then return nil, "连拍记录格式无效" end
	local burst_id = last.burst
	local files = {}
	for index = #entries, 1, -1 do
		local entry = entries[index]
		if type(entry) ~= "table" then break end
		if burst_id ~= nil and entry.burst ~= burst_id then break end
		if type(entry.path) == "string" then
			table.insert(files, 1, entry.path)
		end
	end
	if #files < 2 then
		return nil, "最近的连拍不足以拼接（至少需要两张）"
	end
	return files
end

local function mpv_quality_args(format, quality)
	quality = quality or {}
	if format == "png" then
		return {
			"--ovcopts=compression_level="
				.. clamp_int(quality.png_compression, 0, 9, 4),
		}
	elseif format == "jpg" then
		return { "--ovcopts=qscale=" .. jpeg_qscale(quality.jpeg_quality) }
	elseif format == "webp" then
		if quality.webp_lossless then
			return { "--ovcopts=lossless=1" }
		end
		return {
			"--ovcopts=quality=" .. clamp_int(quality.webp_quality, 0, 100, 75)
				.. ",compression_level="
				.. clamp_int(quality.webp_compression, 0, 6, 4),
		}
	end
	return {}
end

local function ffmpeg_quality_args(format, quality)
	quality = quality or {}
	if format == "png" then
		return {
			"-c:v", "png",
			"-compression_level",
			tostring(clamp_int(quality.png_compression, 0, 9, 4)),
		}
	elseif format == "jpg" then
		return {
			"-c:v", "mjpeg",
			"-q:v", tostring(jpeg_qscale(quality.jpeg_quality)),
		}
	elseif format == "webp" then
		local args = {
			"-c:v", "libwebp",
			"-lossless", quality.webp_lossless and "1" or "0",
		}
		if not quality.webp_lossless then
			args[#args + 1] = "-quality"
			args[#args + 1] = tostring(clamp_int(quality.webp_quality, 0, 100, 75))
		end
		args[#args + 1] = "-compression_level"
		args[#args + 1] = tostring(clamp_int(quality.webp_compression, 0, 6, 4))
		return args
	end
	return {}
end

local function validate_inputs(inputs)
	if type(inputs) ~= "table" or #inputs < 2 then
		return nil, "拼接至少需要两张图片"
	end
	for index = 1, #inputs do
		if type(inputs[index]) ~= "string" or inputs[index] == "" then
			return nil, "拼接图片路径无效"
		end
	end
	return true
end

-- ---------------------------------------------------------------------------
-- Image header probing.  hstack needs equal heights and vstack equal widths;
-- when the inputs disagree, every image is scaled to the largest common edge
-- with the aspect ratio preserved.  Only the formats this script writes are
-- recognized; unknown headers simply disable scaling.
-- ---------------------------------------------------------------------------

local PNG_SIGNATURE = "\137PNG\r\n\26\n"

local function uint16_be(data, position)
	local high, low = data:byte(position, position + 1)
	if not high or not low then return nil end
	return high * 256 + low
end

local function uint32_be(data, position)
	local a, b, c, d = data:byte(position, position + 3)
	if not a or not b or not c or not d then return nil end
	return ((a * 256 + b) * 256 + c) * 256 + d
end

local function png_size(data)
	if #data < 24 or data:sub(1, 8) ~= PNG_SIGNATURE then return nil end
	if data:sub(13, 16) ~= "IHDR" then return nil end
	local width = uint32_be(data, 17)
	local height = uint32_be(data, 21)
	if not width or not height or width == 0 or height == 0 then return nil end
	return width, height
end

-- Walk the JPEG marker segments until a start-of-frame marker is found.
local function jpeg_size(data)
	if data:sub(1, 2) ~= "\255\216" then return nil end
	local position = 3
	while position + 1 <= #data do
		if data:byte(position) ~= 0xFF then return nil end
		while data:byte(position + 1) == 0xFF do
			position = position + 1
		end
		local marker = data:byte(position + 1)
		if not marker then return nil end
		local segment = position + 2
		local has_length = not (marker == 0xD8 or marker == 0xD9
			or marker == 0x01 or (marker >= 0xD0 and marker <= 0xD7))
		if has_length then
			local length = uint16_be(data, segment)
			if not length or length < 2 then return nil end
			local is_sof = marker >= 0xC0 and marker <= 0xCF
				and marker ~= 0xC4 and marker ~= 0xC8 and marker ~= 0xCC
			if is_sof then
				local height = uint16_be(data, segment + 3)
				local width = uint16_be(data, segment + 5)
				if not width or not height or width == 0 or height == 0 then
					return nil
				end
				return width, height
			end
			position = segment + length
		else
			position = segment
		end
	end
	return nil
end

local function webp_size(data)
	if #data < 30 or data:sub(1, 4) ~= "RIFF" or data:sub(9, 12) ~= "WEBP" then
		return nil
	end
	local chunk = data:sub(13, 16)
	-- The chunk header is 8 bytes (four-character tag plus 32-bit size).
	local payload = 21
	local function uint16_le(position)
		local low, high = data:byte(position, position + 1)
		if not low or not high then return nil end
		return low + high * 256
	end
	local function uint32_le(position)
		local a, b, c, d = data:byte(position, position + 3)
		if not a or not b or not c or not d then return nil end
		return ((d * 256 + c) * 256 + b) * 256 + a
	end
	local function uint24_le(position)
		local a, b, c = data:byte(position, position + 2)
		if not a or not b or not c then return nil end
		return a + b * 256 + c * 65536
	end
	if chunk == "VP8 " then
		if #data < payload + 9 then return nil end
		if data:byte(payload + 3) ~= 0x9D or data:byte(payload + 4) ~= 0x01
			or data:byte(payload + 5) ~= 0x2A then
			return nil
		end
		local width = uint16_le(payload + 6) % 16384
		local height = uint16_le(payload + 8) % 16384
		if not width or not height or width == 0 or height == 0 then return nil end
		return width, height
	elseif chunk == "VP8L" then
		if #data < payload + 5 or data:byte(payload) ~= 0x2F then return nil end
		local value = uint32_le(payload + 1)
		if not value then return nil end
		local width = 1 + value % 16384
		local height = 1 + math.floor(value / 16384) % 16384
		if width == 0 or height == 0 then return nil end
		return width, height
	elseif chunk == "VP8X" then
		local width = uint24_le(payload + 4)
		local height = uint24_le(payload + 7)
		if not width or not height then return nil end
		return 1 + width, 1 + height
	end
	return nil
end

--- Extract image dimensions from the leading bytes of a file.
-- Returns width, height, or nil for unknown/incomplete headers.
local function parse_image_size(data)
	if type(data) ~= "string" then return nil end
	if data:sub(1, 8) == PNG_SIGNATURE then return png_size(data) end
	if data:sub(1, 2) == "\255\216" then return jpeg_size(data) end
	if data:sub(1, 4) == "RIFF" and data:sub(9, 12) == "WEBP" then
		return webp_size(data)
	end
	return nil
end

local IMAGE_PROBE_BYTES = 262144

local function read_image_size(path)
	local file = io.open(path, "rb")
	if not file then return nil end
	local data = file:read(IMAGE_PROBE_BYTES)
	file:close()
	local width, height = parse_image_size(data)
	if not width or not height then return nil end
	return { w = width, h = height }
end

-- Return one scale filter per input, or nil when all images already share
-- the stacking dimension or when the sizes are unknown.
local function stitch_scale_specs(sizes, direction)
	direction = normalize_direction(direction)
	if not direction or type(sizes) ~= "table" or #sizes < 2 then return nil end
	local function dimension(size)
		if type(size) ~= "table" then return nil end
		local width, height = tonumber(size.w), tonumber(size.h)
		if not width or not height or width <= 0 or height <= 0 then return nil end
		if direction == "vstack" then return width end
		return height
	end
	local first = dimension(sizes[1])
	if not first then return nil end
	local uniform = true
	local target = 0
	for index = 1, #sizes do
		local value = dimension(sizes[index])
		if not value then return nil end
		if value ~= first then uniform = false end
		if value > target then target = value end
	end
	if uniform then return nil end
	-- The yuv420p encoders used by mpv and ffmpeg need even dimensions.
	target = math.floor(target / 2) * 2
	if target < 2 then target = 2 end
	local specs = {}
	for index = 1, #sizes do
		if dimension(sizes[index]) ~= target then
			if direction == "vstack" then
				specs[index] = string.format("scale=%d:-2", target)
			else
				specs[index] = string.format("scale=-2:%d", target)
			end
		end
	end
	return specs
end

local MPV_CODECS = { png = "png", jpg = "mjpeg", webp = "libwebp" }

local function build_mpv_stitch_args(job)
	if type(job) ~= "table" then return nil, "拼接参数无效" end
	if type(job.mpv) ~= "string" or job.mpv == "" then
		return nil, "未指定 mpv 可执行文件"
	end
	if type(job.output) ~= "string" or job.output == "" then
		return nil, "未指定拼接输出文件"
	end
	local ok, err = validate_inputs(job.inputs)
	if not ok then return nil, err end
	local direction = normalize_direction(job.direction)
	if not direction then return nil, "拼接方向必须是 hstack 或 vstack" end
	local format = normalize_format(job.format)
	if not format then return nil, "拼接格式必须是 png、jpg 或 webp" end

	local specs = stitch_scale_specs(job.sizes, direction)
	local chains = {}
	local inputs = {}
	for index = 1, #job.inputs do
		local label = "vid" .. index
		if specs and specs[index] then
			chains[#chains + 1] = "[vid" .. index .. "]" .. specs[index]
				.. "[sv" .. index .. "]"
			label = "sv" .. index
		end
		inputs[index] = "[" .. label .. "]"
	end
	local graph = table.concat(inputs, "")
		.. direction .. "=inputs=" .. #job.inputs .. "[vo]"
	if #chains > 0 then
		graph = table.concat(chains, ";") .. ";" .. graph
	end

	local args = {
		job.mpv,
		"--no-config",
		"--no-terminal",
		"--no-audio",
		"--no-osc",
		"--no-input-default-bindings",
		"--msg-level=all=error",
		"--frames=1",
		"--of=image2",
		"--o=" .. job.output,
		"--ovc=" .. MPV_CODECS[format],
		"--ofopts=update=1",
	}
	for _, item in ipairs(mpv_quality_args(format, job.quality)) do
		args[#args + 1] = item
	end
	args[#args + 1] = "--lavfi-complex=" .. graph
	-- mpv treats positional files as playlist entries.  Extra images become
	-- simultaneous tracks through --external-file and are addressed as vid2,
	-- vid3, ... by the complex graph.
	for index = 2, #job.inputs do
		args[#args + 1] = "--external-file=" .. job.inputs[index]
	end
	args[#args + 1] = job.inputs[1]
	return args
end

local function build_ffmpeg_stitch_args(job)
	if type(job) ~= "table" then return nil, "拼接参数无效" end
	if type(job.ffmpeg) ~= "string" or job.ffmpeg == "" then
		return nil, "未指定 ffmpeg 可执行文件"
	end
	if type(job.output) ~= "string" or job.output == "" then
		return nil, "未指定拼接输出文件"
	end
	local ok, err = validate_inputs(job.inputs)
	if not ok then return nil, err end
	local direction = normalize_direction(job.direction)
	if not direction then return nil, "拼接方向必须是 hstack 或 vstack" end
	local format = normalize_format(job.format)
	if not format then return nil, "拼接格式必须是 png、jpg 或 webp" end

	local specs = stitch_scale_specs(job.sizes, direction)
	local chains = {}
	local inputs = {}
	for index = 0, #job.inputs - 1 do
		local label = index .. ":v"
		if specs and specs[index + 1] then
			chains[#chains + 1] = "[" .. label .. "]" .. specs[index + 1]
				.. "[sv" .. index .. "]"
			label = "sv" .. index
		end
		inputs[#inputs + 1] = "[" .. label .. "]"
	end
	local graph = table.concat(inputs, "")
		.. direction .. "=inputs=" .. #job.inputs .. "[v]"
	if #chains > 0 then
		graph = table.concat(chains, ";") .. ";" .. graph
	end

	local args = {
		job.ffmpeg,
		"-hide_banner",
		"-nostdin",
		"-loglevel", "error",
		"-y",
	}
	for _, input in ipairs(job.inputs) do
		args[#args + 1] = "-i"
		args[#args + 1] = input
	end
	args[#args + 1] = "-filter_complex"
	args[#args + 1] = graph
	args[#args + 1] = "-map"
	args[#args + 1] = "[v]"
	args[#args + 1] = "-frames:v"
	args[#args + 1] = "1"
	for _, item in ipairs(ffmpeg_quality_args(format, job.quality)) do
		args[#args + 1] = item
	end
	args[#args + 1] = "-update"
	args[#args + 1] = "1"
	args[#args + 1] = job.output
	return args
end

-- ---------------------------------------------------------------------------
-- Runtime helpers
-- ---------------------------------------------------------------------------

local state = {
	burst = nil,
	stitch = nil,
}

local active_capture = nil
local queue_warned = false
local burst_counter = 0

local function is_absolute_path(path)
	return path:sub(1, 1) == "/"
		or path:match("^%a:[/\\]") ~= nil
		or path:sub(1, 2) == "\\\\"
end

-- mpv only expands the special "~~" keywords when they are followed by a
-- slash: "~~desktop" is returned verbatim while "~~desktop/" is resolved.
-- With --no-config the config-directory keywords expand to an empty string,
-- so callers must treat "" as "not available" instead of using it as a path.
local function expand_path(value)
	local text = trim(value)
	if text == "" then return "" end
	local query = text
	if query:match("^~~[%w_]*$") or query == "~" then
		query = query .. "/"
	end
	local ok, expanded = pcall(mp.command_native, { "expand-path", query })
	if ok and type(expanded) == "string" then
		return expanded
	end
	return text
end

local function absolute_path(value)
	local path = expand_path(value)
	if path == "" then return "" end
	if is_absolute_path(path) then return path end
	local cwd = utils.getcwd() or "."
	return utils.join_path(cwd, path)
end

-- The user layer normally lives at <config>/user.  With --no-config mpv has
-- no config directory and expands ~~/user to the relative path "user"; keep
-- the same layout below the working directory so queues and screenshots still
-- work instead of failing with a relative containment check.
local function user_layer_dir()
	local dir = expand_path("~~/user")
	if dir == "" then dir = "user" end
	if not is_absolute_path(dir) then
		return utils.join_path(utils.getcwd() or ".", dir)
	end
	return dir
end

local function queue_path()
	local user_root = user_layer_dir()
	local configured = absolute_path(settings.queue_file)
	if configured ~= "" and is_inside(configured, user_root) then
		return configured
	end
	if not queue_warned then
		queue_warned = true
		msg.warn("screenshot_tools: queue_file must stay inside the user layer; using the default")
	end
	return utils.join_path(user_root, "screenshot-tools/queue.jsonl")
end

local function output_dir()
	local dir = absolute_path(settings.directory)
	if dir == "" then
		dir = utils.join_path(user_layer_dir(), "screenshots")
	end
	local config_root = expand_path("~~")
	local user_root = user_layer_dir()
	if config_root ~= "" and is_inside(dir, config_root) and not is_inside(dir, user_root) then
		return nil, "截图目录不能位于播放器配置目录内（user/ 除外）"
	end
	return dir
end

local function ensure_dir(path)
	if path == nil or path == "" then return false end
	local info = utils.file_info(path)
	if info and info.is_dir then return true end

	local args
	if platform == "windows" then
		-- cmd.exe is required because mkdir is a shell builtin on Windows.
		-- The path is still a separate argv element, never a concatenated
		-- command line.
		args = { "cmd.exe", "/d", "/c", "mkdir", path }
	else
		args = { "mkdir", "-p", "--", path }
	end
	local ok, result = pcall(mp.command_native, {
		name = "subprocess",
		args = args,
		playback_only = false,
		capture_stdout = true,
		capture_stderr = true,
	})
	if not ok or type(result) ~= "table" or result.status ~= 0 then
		msg.warn("screenshot_tools: could not create directory: " .. tostring(path))
		return false
	end
	info = utils.file_info(path)
	return info ~= nil and info.is_dir == true
end

local function unique_output_path(dir, name)
	local candidate = utils.join_path(dir, name)
	if utils.file_info(candidate) == nil then return candidate end
	local stem, extension = name:match("^(.*)(%.[^%.]+)$")
	if stem == nil then
		stem, extension = name, ""
	end
	for index = 2, 999 do
		candidate = utils.join_path(
			dir,
			string.format("%s-%d%s", stem, index, extension)
		)
		if utils.file_info(candidate) == nil then return candidate end
	end
	return nil
end

local function current_video_size()
	local width = mp.get_property_number("video-params/w", 0) or 0
	local height = mp.get_property_number("video-params/h", 0) or 0
	if width <= 0 or height <= 0 then
		width = mp.get_property_number("width", 0) or 0
		height = mp.get_property_number("height", 0) or 0
	end
	return width, height
end

local function osd(text, duration)
	mp.osd_message(tostring(text), duration or 3000)
end

local function append_queue(entry)
	local path = queue_path()
	if path == "" then return false end
	local dir = utils.split_path(path)
	if dir and dir ~= "" and not ensure_dir(dir) then return false end
	local file, err = io.open(path, "ab")
	if not file then
		msg.warn("screenshot_tools: could not open queue file: " .. tostring(err))
		return false
	end
	local encoded = utils.format_json(entry)
	if type(encoded) ~= "string" then
		file:close()
		msg.warn("screenshot_tools: could not encode queue entry")
		return false
	end
	file:write(encoded, "\n")
	file:close()
	return true
end

local function read_queue_entries()
	local entries = {}
	local path = queue_path()
	if path == "" then return entries end
	local file = io.open(path, "rb")
	if not file then return entries end
	for line in file:lines() do
		line = trim(line)
		if line ~= "" then
			local ok, entry = pcall(utils.parse_json, line)
			if ok and type(entry) == "table" and type(entry.path) == "string" then
				entries[#entries + 1] = entry
			end
		end
	end
	file:close()
	return entries
end

--- Write one screenshot and call callback(ok, path_or_error).
-- When a region is supplied, video-crop is set before the capture and restored
-- after the image file actually appears on disk.  screenshot-to-file is
-- asynchronous in mpv, so polling the target path is the only reliable
-- completion signal available to a Lua script.
local function issue_capture(job, callback)
	if active_capture ~= nil then
		callback(false, "另一个截图操作尚未完成")
		return false
	end
	local format = normalize_format(job.format or settings.format)
	if not format then
		callback(false, "截图格式必须是 png、jpg 或 webp")
		return false
	end
	local mode = normalize_mode(job.mode or settings.default_mode)
	if not mode then
		callback(false, "截图模式必须是 video、subtitles 或 window")
		return false
	end
	local dir, dir_err = output_dir()
	if not dir then
		callback(false, dir_err)
		return false
	end
	if not ensure_dir(dir) then
		callback(false, "无法创建截图目录：" .. dir)
		return false
	end

	local stamp = os.date("%Y%m%d-%H%M%S")
	local name, name_err = build_filename(
		settings.filename_prefix, format, mode, stamp, job.index
	)
	if not name then
		callback(false, name_err)
		return false
	end
	local path = unique_output_path(dir, name)
	if not path then
		callback(false, "无法生成唯一的截图文件名")
		return false
	end

	local original_crop = nil
	if job.region ~= nil then
		local width, height = current_video_size()
		local valid, region_err = validate_region(job.region, width, height)
		if not valid then
			callback(false, region_err)
			return false
		end
		original_crop = mp.get_property("video-crop") or ""
		local invoked, applied, apply_err = pcall(
			mp.set_property, "video-crop", format_crop(job.region)
		)
		if not invoked or not applied then
			local detail = apply_err
			if not invoked then detail = applied end
			callback(false, "无法应用截图区域：" .. tostring(detail or "未知错误"))
			return false
		end
	end

	local context = {
		finished = false,
		timers = {},
		path = path,
		original_crop = original_crop,
	}
	active_capture = context

	local function stop_timers()
		for _, timer in ipairs(context.timers) do
			pcall(timer.kill, timer)
		end
		context.timers = {}
	end

	local function finish(ok, detail)
		if context.finished then return end
		context.finished = true
		stop_timers()
		if context.original_crop ~= nil then
			pcall(mp.set_property, "video-crop", context.original_crop)
			context.original_crop = nil
		end
		if active_capture == context then
			active_capture = nil
		end
		callback(ok, detail)
	end

	context.cancel = function(reason)
		finish(false, reason or "已取消")
	end

	local function schedule(delay, fn)
		if context.finished then return end
		local timer
		timer = mp.add_timeout(delay, function()
			timer = nil
			if not context.finished then fn() end
		end)
		context.timers[#context.timers + 1] = timer
		return timer
	end

	local function wait_for_file(deadline)
		if context.finished then return end
		local info = utils.file_info(path)
		if info and info.is_file and (info.size == nil or info.size > 0) then
			finish(true, path)
			return
		end
		if mp.get_time() >= deadline then
			finish(false, "等待截图文件超时：" .. path)
			return
		end
		schedule(0.05, function()
			wait_for_file(deadline)
		end)
	end

	local function take()
		if context.finished then return end
		local invoked, result, command_error = pcall(mp.command_native, {
			"screenshot-to-file", path, mode,
		})
		if not invoked then
			finish(false, "截图命令失败：" .. tostring(result))
			return
		end
		if result == nil and command_error ~= nil then
			finish(false, "截图命令失败：" .. tostring(command_error))
			return
		end
		local timeout = math.max(1, math.min(120, number_or(settings.file_timeout, 10)))
		wait_for_file(mp.get_time() + timeout)
	end

	local delay = math.max(0, math.min(5, number_or(settings.capture_delay, 0.2)))
	local function start_capture()
		if job.region == nil then
			if delay > 0 then
				schedule(delay, take)
			else
				take()
			end
			return
		end
		-- The VO applies video-crop asynchronously.  Wait until the rendered
		-- size reflects the requested rectangle.  A fixed window may never
		-- match, so fall back to a screenshot after one extra second instead
		-- of risking a timeout.
		local started_at = mp.get_time()
		local deadline = started_at + delay + 1.0
		local function wait_crop()
			if context.finished then return end
			local width = mp.get_property_number("osd-dimensions/w", 0) or 0
			local height = mp.get_property_number("osd-dimensions/h", 0) or 0
			local applied = width == job.region.w and height == job.region.h
			local elapsed = mp.get_time() - started_at
			if (applied and elapsed >= delay) or mp.get_time() >= deadline then
				take()
			else
				schedule(0.02, wait_crop)
			end
		end
		wait_crop()
	end
	local started, start_err = pcall(start_capture)
	if not started then
		finish(false, "无法启动截图：" .. tostring(start_err))
		return false
	end
	return true
end

local function capture(mode, region)
	return issue_capture({ mode = mode, region = region }, function(ok, detail)
		if ok then
			osd("已保存截图：" .. detail, 3000)
		elseif detail == "已取消" then
			osd("已取消截图", 2000)
		elseif detail == "文件已切换" then
			osd("文件已切换，截图已取消", 2500)
		else
			msg.error("screenshot_tools: " .. tostring(detail))
			osd("截图失败：" .. tostring(detail), 5000)
		end
	end)
end

local function cancel_active_capture(reason)
	if active_capture ~= nil then
		active_capture.cancel(reason or "已取消")
		return true
	end
	return false
end

--- Stop the current burst, restore the paused playback position and report.
local function stop_burst(message, duration)
	local burst = state.burst
	if burst == nil then return false end
	state.burst = nil
	burst.cancelled = true
	for _, timer in ipairs(burst.timers) do
		pcall(timer.kill, timer)
	end
	burst.timers = {}
	cancel_active_capture("已取消连拍")
	if burst.paused and burst.original_time ~= nil then
		pcall(mp.commandv, "seek", burst.original_time, "absolute", "exact")
	end
	if message then osd(message, duration or 4000) end
	return true
end

local function handler_burst(args)
	if state.burst ~= nil then
		osd("连拍正在进行中", 3000)
		return
	end
	local parsed, parse_err = parse_burst_args(args, {
		count = settings.burst_count,
		interval = settings.burst_interval,
		mode = settings.burst_mode,
	})
	if not parsed then
		osd("连拍参数无效：" .. tostring(parse_err), 5000)
		return
	end
	local max_count = math.max(
		1, math.min(1000, math.floor(number_or(settings.burst_max_count, 200)))
	)
	if parsed.count > max_count then
		msg.warn(string.format(
			"screenshot_tools: burst count %d exceeds the configured maximum; using %d",
			parsed.count, max_count
		))
		parsed.count = max_count
	end
	local start = mp.get_property_number("time-pos")
	if start == nil then
		osd("当前没有可截图的视频", 3000)
		return
	end
	local times, times_err = compute_burst_times(
		start, parsed.interval, parsed.count
	)
	if not times then
		osd("连拍参数无效：" .. tostring(times_err), 5000)
		return
	end
	local mode = normalize_mode(parsed.mode) or "subtitles"
	local format = normalize_format(settings.format)
	if not format then
		osd("截图格式必须是 png、jpg 或 webp", 4000)
		return
	end
	local dir, dir_err = output_dir()
	if not dir then
		osd("连拍失败：" .. tostring(dir_err), 5000)
		return
	end
	if not ensure_dir(dir) then
		osd("连拍失败：无法创建截图目录：" .. dir, 5000)
		return
	end

	burst_counter = (burst_counter + 1) % 1000
	local burst = {
		id = os.date("%Y%m%d-%H%M%S") .. string.format("-%03d", burst_counter),
		mode = mode,
		format = format,
		times = times,
		count = #times,
		index = 0,
		done = 0,
		failures = 0,
		cancelled = false,
		paused = mp.get_property_bool("pause") == true,
		original_time = start,
		timers = {},
	}
	state.burst = burst

	local function schedule(delay, fn)
		if burst.cancelled then return end
		local timer = mp.add_timeout(delay, function()
			if not burst.cancelled then fn() end
		end)
		burst.timers[#burst.timers + 1] = timer
		return timer
	end

	local function finish()
		if state.burst ~= burst then return end
		local failures = burst.failures
		local done = burst.done
		stop_burst(string.format(
			"连拍完成：成功 %d 张，失败 %d 张", done - failures, failures
		), 5000)
	end

	local next_frame

	local function capture_frame(target)
		if burst.cancelled then return end
		local started = issue_capture({
			mode = burst.mode,
			format = burst.format,
			index = burst.index,
		}, function(ok, detail)
			if burst.cancelled or state.burst ~= burst then return end
			if ok then
				append_queue({
					burst = burst.id,
					index = burst.index,
					path = detail,
					time = target,
					mode = burst.mode,
					format = burst.format,
					created = os.time(),
				})
			else
				burst.failures = burst.failures + 1
				msg.warn("screenshot_tools: burst frame failed: " .. tostring(detail))
			end
			burst.done = burst.done + 1
			if burst.mode ~= "window" then
				osd(string.format("连拍 %d/%d", burst.done, burst.count), 800)
			end
			schedule(0, next_frame)
		end)
		if started == false and not burst.cancelled then
			-- issue_capture already reported through the callback.
			return
		end
	end

	local function stop_early(reason)
		if state.burst ~= burst then return end
		stop_burst(string.format(
			"连拍提前结束：%s（成功 %d 张）", reason, burst.done - burst.failures
		), 5000)
	end

	-- Return a reason when playback can no longer reach the scheduled time.
	-- Without this check a target past EOF would wait forever because time-pos
	-- stops advancing.
	local function playback_problem()
		if mp.get_property_bool("eof-reached", false) then
			return "视频已播放完毕"
		end
		if mp.get_property_number("time-pos") == nil then
			return "视频已结束"
		end
		if (mp.get_property_bool("pause") == true) ~= burst.paused then
			return "播放状态已改变"
		end
		return nil
	end

	local function capture_paused(target)
		pcall(mp.commandv, "seek", target, "absolute", "exact")
		local deadline = mp.get_time() + 5
		local function wait_seek()
			if burst.cancelled then return end
			local current = mp.get_property_number("time-pos")
			if current == nil then
				stop_early("视频已结束")
				return
			end
			local seeking = mp.get_property_bool("seeking")
			local arrived = math.abs(current - target) <= 0.05
			if (arrived and seeking ~= true) or mp.get_time() >= deadline then
				local delay = math.max(
					0, math.min(5, number_or(settings.capture_delay, 0.2))
				)
				schedule(delay, function()
					capture_frame(target)
				end)
			else
				schedule(0.05, wait_seek)
			end
		end
		schedule(0.05, wait_seek)
	end

	local function capture_playing(target)
		local problem = playback_problem()
		if problem then
			stop_early(problem)
			return
		end
		local now = mp.get_property_number("time-pos")
		if now == nil then now = burst.original_time end
		local speed = mp.get_property_number("speed", 1) or 1
		if speed <= 0 then speed = 1 end
		local wait = (target - now) / speed
		if wait <= 0.02 then
			capture_frame(target)
		else
			schedule(wait, function()
				local later = playback_problem()
				if later then
					stop_early(later)
					return
				end
				capture_frame(target)
			end)
		end
	end

	next_frame = function()
		if burst.cancelled or state.burst ~= burst then return end
		if burst.index >= burst.count then
			finish()
			return
		end
		burst.index = burst.index + 1
		local target = burst.times[burst.index]
		if burst.paused then
			capture_paused(target)
		else
			capture_playing(target)
		end
	end

	next_frame()
end

local function collect_quality()
	return {
		jpeg_quality = mp.get_property_number("screenshot-jpeg-quality", 100) or 100,
		png_compression = mp.get_property_number("screenshot-png-compression", 4) or 4,
		webp_lossless = mp.get_property_bool("screenshot-webp-lossless", false) == true,
		webp_quality = mp.get_property_number("screenshot-webp-quality", 75) or 75,
		webp_compression = mp.get_property_number("screenshot-webp-compression", 4) or 4,
	}
end

local function finish_stitch(path)
	local info = utils.file_info(path)
	if info and info.is_file and (info.size == nil or info.size > 0) then
		osd("已拼接图片：" .. path, 4000)
		return true
	end
	return false
end

local function fail_stitch(detail)
	local text = tostring(detail or "未知错误")
	if #text > 600 then text = text:sub(1, 600) .. "…" end
	msg.error("screenshot_tools: stitch failed: " .. text)
	osd("拼接失败：" .. text, 6000)
end

--- Run a subprocess asynchronously with a hard timeout.
-- callback(ok, detail, cancelled)
local function run_subprocess_async(tool, args, timeout, callback)
	local context = { done = false, cancelled = false, handle = nil, timer = nil }

	local function finish(ok, detail, cancelled)
		if context.done then return end
		context.done = true
		context.cancelled = cancelled == true
		if context.timer ~= nil then
			pcall(context.timer.kill, context.timer)
			context.timer = nil
		end
		if state.stitch == context then
			state.stitch = nil
		end
		callback(ok, detail, context.cancelled)
	end

	context.cancel = function(reason)
		if context.done then return end
		if context.handle ~= nil then
			pcall(mp.abort_async_command, context.handle)
		end
		finish(false, reason or "已取消", true)
	end

	context.timer = mp.add_timeout(timeout, function()
		if context.handle ~= nil then
			pcall(mp.abort_async_command, context.handle)
		end
		finish(false, tool .. " 超时", false)
	end)
	state.stitch = context

	local ok, handle = pcall(mp.command_native_async, {
		name = "subprocess",
		args = args,
		playback_only = false,
		capture_stdout = true,
		capture_stderr = true,
	}, function(success, result, error)
		if not success then
			finish(false, "无法启动 " .. tool, false)
			return
		end
		if type(result) ~= "table" or result.status ~= 0 then
			local detail = error
			if type(result) == "table" then
				detail = result.stderr or result.error or detail
			end
			finish(false, detail or (tool .. " 返回非零状态"), false)
			return
		end
		finish(true, "ok", false)
	end)
	if not ok or handle == nil then
		finish(false, "无法启动 " .. tool, false)
		return
	end
	context.handle = handle
end

local function bundled_mpv_path()
	local configured = trim(settings.mpv_path)
	if configured ~= "" then
		local path = absolute_path(configured)
		local info = utils.file_info(path)
		if info and info.is_file then return path end
		msg.warn("screenshot_tools: configured mpv_path is not a file: " .. configured)
		return nil
	end
	local candidates
	if platform == "windows" then
		candidates = { "~~/../mpv.exe" }
	else
		-- The packaged AppImage is unpacked to mpv/; bin/mpv is the real
		-- binary, and AppRun is the entry point used when the layout differs.
		candidates = { "~~/../mpv/bin/mpv", "~~/../mpv/AppRun" }
	end
	for _, candidate in ipairs(candidates) do
		local path = absolute_path(candidate)
		local info = utils.file_info(path)
		if info and info.is_file then return path end
	end
	return nil
end

local function ffmpeg_executable()
	local configured = trim(settings.ffmpeg_path)
	if configured == "" then configured = "ffmpeg" end
	if configured:find("[/\\]") or configured:match("^%a:") then
		local path = absolute_path(configured)
		local info = utils.file_info(path)
		if info and info.is_file then return path end
		return nil
	end
	local ok, result = pcall(mp.command_native, {
		name = "subprocess",
		args = { configured, "-version" },
		playback_only = false,
		capture_stdout = true,
		capture_stderr = true,
	})
	if ok and type(result) == "table" and result.status == 0 then
		return configured
	end
	return nil
end

local function start_stitch(job)
	local timeout = math.max(
		5, math.min(3600, number_or(settings.subprocess_timeout, 120))
	)

	local function try_ffmpeg(previous_error)
		local ffmpeg = ffmpeg_executable()
		if not ffmpeg then
			local detail = "未找到随包 mpv，且 PATH 中没有可用的 ffmpeg"
			if previous_error ~= nil and previous_error ~= "" then
				detail = detail .. "（mpv: " .. tostring(previous_error) .. "）"
			end
			fail_stitch(detail)
			return
		end
		local args, args_err = build_ffmpeg_stitch_args({
			ffmpeg = ffmpeg,
			inputs = job.inputs,
			output = job.output,
			direction = job.direction,
			format = job.format,
			quality = job.quality,
			sizes = job.sizes,
		})
		if not args then
			fail_stitch(args_err)
			return
		end
		osd("正在使用 ffmpeg 拼接…", 1500)
		run_subprocess_async("ffmpeg", args, timeout, function(ok, detail, cancelled)
			if cancelled then
				osd("已取消拼接", 2000)
			elseif ok and finish_stitch(job.output) then
				-- finish_stitch already reported success.
			elseif ok then
				fail_stitch("ffmpeg 未生成输出文件：" .. job.output)
			else
				fail_stitch("ffmpeg: " .. tostring(detail))
			end
		end)
	end

	local mpv_path = bundled_mpv_path()
	if mpv_path == nil then
		try_ffmpeg(nil)
		return
	end
	local args, args_err = build_mpv_stitch_args({
		mpv = mpv_path,
		inputs = job.inputs,
		output = job.output,
		direction = job.direction,
		format = job.format,
		quality = job.quality,
		sizes = job.sizes,
	})
	if not args then
		try_ffmpeg(args_err)
		return
	end
	osd("正在拼接图片…", 1500)
	run_subprocess_async("mpv", args, timeout, function(ok, detail, cancelled)
		if cancelled then
			osd("已取消拼接", 2000)
		elseif ok and finish_stitch(job.output) then
			-- finish_stitch already reported success.
		elseif ok then
			try_ffmpeg("未生成输出文件")
		else
			try_ffmpeg(detail)
		end
	end)
end

local function resolve_input_path(value)
	local path = absolute_path(value)
	local info = utils.file_info(path)
	if info and info.is_file then return path end
	return nil
end

local function handler_stitch(args)
	if state.stitch ~= nil then
		osd("拼接正在进行中", 3000)
		return
	end
	local direction = normalize_direction(settings.stitch_direction) or "hstack"
	local index = 1
	if args[1] ~= nil then
		local explicit = normalize_direction(args[1])
		if explicit then
			direction = explicit
			index = 2
		end
	end
	local inputs = {}
	for position = index, #args do
		local path = resolve_input_path(args[position])
		if not path then
			osd("找不到待拼接图片：" .. tostring(args[position]), 5000)
			return
		end
		inputs[#inputs + 1] = path
	end
	if #inputs == 0 then
		local files, queue_err = latest_burst(read_queue_entries())
		if not files then
			osd("无法拼接：" .. tostring(queue_err), 5000)
			return
		end
		for _, path in ipairs(files) do
			if not resolve_input_path(path) then
				osd("找不到待拼接图片：" .. tostring(path), 5000)
				return
			end
		end
		inputs = files
	end
	if #inputs < 2 then
		osd("拼接至少需要两张图片", 4000)
		return
	end
	local format = normalize_format(settings.format)
	if not format then
		osd("截图格式必须是 png、jpg 或 webp", 4000)
		return
	end
	local dir, dir_err = output_dir()
	if not dir then
		osd("拼接失败：" .. tostring(dir_err), 5000)
		return
	end
	if not ensure_dir(dir) then
		osd("拼接失败：无法创建输出目录：" .. dir, 5000)
		return
	end
	local name, name_err = build_filename(
		settings.filename_prefix, format, "stitch", os.date("%Y%m%d-%H%M%S")
	)
	if not name then
		osd("拼接失败：" .. tostring(name_err), 5000)
		return
	end
	local output = unique_output_path(dir, name)
	if not output then
		osd("拼接失败：无法生成唯一的输出文件名", 5000)
		return
	end
	-- Probe the headers so mixed-size inputs can be scaled to a common edge
	-- without stretching.  Any unknown format disables scaling entirely and
	-- leaves the stitcher's own size handling in charge.
	local sizes = {}
	local sized = true
	for index = 1, #inputs do
		sizes[index] = read_image_size(inputs[index])
		if not sizes[index] then
			sized = false
			break
		end
	end
	if not sized then sizes = nil end
	start_stitch({
		inputs = inputs,
		output = output,
		direction = direction,
		format = format,
		quality = collect_quality(),
		sizes = sizes,
	})
end

local function handler_capture(args)
	local mode = normalize_mode(args[1] or settings.default_mode)
	if not mode then
		osd("截图模式必须是 video、subtitles 或 window", 4000)
		return
	end
	local region = nil
	if #args > 1 then
		local rest = {}
		for index = 2, #args do rest[#rest + 1] = args[index] end
		local region_err
		region, region_err = parse_region(rest)
		if not region then
			osd("截图区域无效：" .. tostring(region_err), 5000)
			return
		end
	end
	capture(mode, region)
end

local function prompt_region(mode)
	local ok, input = pcall(require, "mp.input")
	if not ok or type(input) ~= "table" or type(input.get) ~= "function" then
		osd("请先配置截图区域（region=x:y:w:h）或传入坐标", 5000)
		return
	end
	input.get({
		prompt = "截图区域 (x:y:w:h)：",
		default = trim(settings.region),
		submit = function(text)
			if text == nil or trim(text) == "" then return end
			local region, region_err = parse_region({ text })
			if not region then
				osd("截图区域无效：" .. tostring(region_err), 5000)
				return
			end
			capture(mode, region)
		end,
	})
end

local function handler_region(args)
	local mode = normalize_mode(settings.default_mode) or "subtitles"
	local start = 1
	if args[1] ~= nil then
		local explicit = normalize_mode(args[1])
		if explicit then
			mode = explicit
			start = 2
		end
	end
	local rest = {}
	for index = start, #args do rest[#rest + 1] = args[index] end
	if #rest == 0 then
		local configured = trim(settings.region)
		if configured == "" then
			prompt_region(mode)
			return
		end
		local region, region_err = parse_region({ configured })
		if not region then
			osd("截图区域无效：" .. tostring(region_err), 5000)
			return
		end
		capture(mode, region)
		return
	end
	local region, region_err = parse_region(rest)
	if not region then
		osd("截图区域无效：" .. tostring(region_err), 5000)
		return
	end
	capture(mode, region)
end

local function handler_video()
	capture("video", nil)
end

local function handler_subtitles()
	capture("subtitles", nil)
end

local function handler_window()
	capture("window", nil)
end

local function handler_cancel()
	local cancelled = false
	if state.burst ~= nil then
		cancelled = stop_burst("已取消连拍", 2000) or cancelled
	end
	if state.stitch ~= nil then
		state.stitch.cancel("已取消拼接")
		cancelled = true
	end
	if cancel_active_capture("已取消") then
		cancelled = true
	end
	if not cancelled then
		osd("没有正在进行的截图任务", 2000)
	end
end

local HANDLERS = {
	capture = handler_capture,
	video = handler_video,
	subtitles = handler_subtitles,
	window = handler_window,
	region = handler_region,
	burst = handler_burst,
	stitch = handler_stitch,
	cancel = handler_cancel,
}

local BINDING_NAMES = {
	"capture",
	"video",
	"subtitles",
	"window",
	"region",
	"burst",
	"stitch",
	"cancel",
}

local M = {
	settings = settings,
	formats = FORMATS,
	modes = MODES,
	directions = DIRECTIONS,
	bindings = BINDING_NAMES,
	sanitize_prefix = sanitize_prefix,
	normalize_format = normalize_format,
	normalize_mode = normalize_mode,
	normalize_direction = normalize_direction,
	build_filename = build_filename,
	parse_region = parse_region,
	validate_region = validate_region,
	format_crop = format_crop,
	normalize_path = normalize_path,
	is_inside = is_inside,
	compute_burst_times = compute_burst_times,
	parse_burst_args = parse_burst_args,
	latest_burst = latest_burst,
	jpeg_qscale = jpeg_qscale,
	mpv_quality_args = mpv_quality_args,
	ffmpeg_quality_args = ffmpeg_quality_args,
	build_mpv_stitch_args = build_mpv_stitch_args,
	build_ffmpeg_stitch_args = build_ffmpeg_stitch_args,
	parse_image_size = parse_image_size,
	read_image_size = read_image_size,
	stitch_scale_specs = stitch_scale_specs,
	expand_path = expand_path,
}

if disabled(settings.enabled) then
	msg.info("screenshot_tools is disabled by configuration")
	return M
end

-- Both key bindings and script messages use the same names, so input.conf and
-- menu.conf can call either `script-binding screenshot_tools/<name>` or
-- `script-message-to screenshot_tools <name>`.
for _, name in ipairs(BINDING_NAMES) do
	local handler = HANDLERS[name]
	mp.add_key_binding(nil, name, function(_, ...)
		handler({ ... })
	end)
	mp.register_script_message(name, function(...)
		handler({ ... })
	end)
end

-- A temporary video-crop belongs to the file it was requested for.  If the
-- file disappears before the asynchronous screenshot finishes, restore the
-- user's crop and stop any burst that would otherwise run on the next file.
mp.register_event("end-file", function()
	if state.burst ~= nil then
		stop_burst("文件已切换，连拍已停止", 3000)
	end
	if active_capture ~= nil then
		active_capture.cancel("文件已切换")
	end
end)

return M
