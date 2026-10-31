-- Persist selected player-wide properties for the portable configuration.
-- This implementation is newly authored for mpv_PlayKit; it does not import
-- code from the historical baseline script.

local mp = require "mp"
local options = require "mp.options"
local utils = require "mp.utils"

local settings = {
	props = "volume,mute",
	file = "~~/saved-props.json",
	save_delay = 0.25,
}
options.read_options(settings)

local function parse_names(value)
	local names, seen = {}, {}
	for name in tostring(value or ""):gmatch("[^,]+") do
		name = name:match("^%s*(.-)%s*$")
		if name ~= "" and not seen[name] then
			names[#names + 1] = name
			seen[name] = true
		end
	end
	return names
end

local properties = parse_names(settings.props)
local state_path = mp.command_native({ "expand-path", settings.file })
local temp_path = state_path .. ".tmp"
local state = {}
local enabled = true
local pending_save

local function read_state()
	local file = io.open(state_path, "rb")
	if not file then return {} end
	local contents = file:read("*a")
	file:close()
	if not contents or contents == "" then return {} end

	local ok, decoded = pcall(utils.parse_json, contents)
	if ok and type(decoded) == "table" then return decoded end
	mp.msg.warn("Saved property data is invalid; starting with defaults")
	return {}
end

local function write_state()
	if not enabled then return end

	local snapshot = {}
	for _, name in ipairs(properties) do
		local value = mp.get_property_native(name)
		local kind = type(value)
		if kind == "boolean" or kind == "number" or kind == "string" then
			snapshot[name] = value
		end
	end

	local encoded = utils.format_json(snapshot)
	if type(encoded) ~= "string" then
		mp.msg.error("Could not serialize saved player properties")
		return
	end

	local file, err = io.open(temp_path, "wb")
	if not file then
		mp.msg.warn("Could not write saved properties: " .. tostring(err))
		return
	end
	file:write(encoded, "\n")
	file:close()

	-- Windows does not replace an existing destination with os.rename().
	os.remove(state_path)
	local ok, rename_err = os.rename(temp_path, state_path)
	if not ok then
		os.remove(temp_path)
		mp.msg.warn("Could not replace saved properties: " .. tostring(rename_err))
		return
	end
	state = snapshot
end

local function schedule_save()
	if not enabled then return end
	if pending_save then pending_save:kill() end
	pending_save = mp.add_timeout(tonumber(settings.save_delay) or 0.25, function()
		pending_save = nil
		write_state()
	end)
end

local function clear_state()
	if pending_save then
		pending_save:kill()
		pending_save = nil
	end
	enabled = false
	state = {}
	os.remove(temp_path)
	local ok, err = os.remove(state_path)
	if not ok and err then
		mp.msg.warn("Could not clear saved properties: " .. tostring(err))
	end
	mp.osd_message("Saved player properties cleared. They remain disabled until restart.", 3000)
end

state = read_state()
for _, name in ipairs(properties) do
	local value = state[name]
	local kind = type(value)
	if kind == "boolean" or kind == "number" or kind == "string" then
		local ok, err = pcall(mp.set_property_native, name, value)
		if not ok then
			mp.msg.warn("Could not restore property '" .. name .. "': " .. tostring(err))
		end
	end
	mp.observe_property(name, "native", function(_, value)
		if enabled and value ~= nil then schedule_save() end
	end)
end

mp.register_event("shutdown", write_state)
mp.register_script_message("clean_data", clear_state)
