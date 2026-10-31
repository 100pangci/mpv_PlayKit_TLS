-- mpv_PlayKit update checker
--
-- This script only *detects* a newer PlayKit release and shows a notice.  It
-- never downloads or replaces the player.  Set `enabled=no` in
-- script-opts/update-check.conf to disable it completely: in that case this
-- script performs no file writes, spawns no subprocess and makes no network
-- request.  Disabling from the in-player menu writes a persistent state file,
-- which suppresses automatic checks until it is reset from the menu.

local mp = require "mp"
local options = require "mp.options"
local utils = require "mp.utils"

local opts = {
	enabled = true,
	repository = "100pangci/mpv_PlayKit_TLS",
	check_interval_hours = 24,
	timeout_seconds = 12,
	snooze_hours = 24,
}
options.read_options(opts, "update-check")

local function truthy_disabled(value)
	return value == false or value == 0 or value == "no" or value == "false" or value == "0"
end

if truthy_disabled(opts.enabled) then
	mp.msg.info("update-check is disabled by configuration; no network request will be made")
	return
end

if type(opts.repository) ~= "string" or not opts.repository:match("^[%w_.%-]+/[%w_.%-]+$") then
	mp.msg.error("update-check: repository must look like owner/name")
	return
end

local state_path = mp.command_native({ "expand-path", "~~/update-check-state.json" })
local version_path = mp.command_native({ "expand-path", "~~/playkit-version.json" })
-- The API check cannot see releases of a private repository without a token,
-- but the user's browser session can.  Keep a release-page fallback so the
-- menu item always opens something useful.
local releases_url = string.format("https://github.com/%s/releases/latest", opts.repository)

local state = {
	schema = 1,
	last_check = 0,
	latest_version = "",
	latest_url = "",
	snoozed_until = 0,
	dismissed_version = "",
	disabled = false,
}

local function read_json_file(path, limit)
	local file = io.open(path, "rb")
	if not file then return nil end
	local contents = file:read(limit or (256 * 1024))
	file:close()
	if not contents or contents == "" then return nil end
	local ok, decoded = pcall(utils.parse_json, contents)
	if ok and type(decoded) == "table" then return decoded end
	return nil
end

local function write_state()
	local encoded = utils.format_json(state)
	if type(encoded) ~= "string" then
		mp.msg.error("update-check: could not serialize state")
		return
	end
	local temp_path = state_path .. ".tmp"
	local file = io.open(temp_path, "wb")
	if not file then
		mp.msg.warn("update-check: could not write state file")
		return
	end
	local written, write_err = file:write(encoded, "\n")
	local closed, close_err = file:close()
	if not written or not closed then
		os.remove(temp_path)
		mp.msg.warn(
			"update-check: could not write state file: "
			.. tostring(write_err or close_err)
		)
		return
	end
	-- Prefer an atomic replace.  On Windows os.rename cannot overwrite an
	-- existing file, so fall back to removing the old state only after the
	-- temporary file is known to be complete.
	local ok, err = os.rename(temp_path, state_path)
	if not ok then
		os.remove(state_path)
		ok, err = os.rename(temp_path, state_path)
	end
	if not ok then
		os.remove(temp_path)
		mp.msg.warn("update-check: could not replace state file: " .. tostring(err))
	end
end

-- The persistent "disabled" state and a missing/non-release version only
-- suppress the automatic check.  Keep the script loaded so the menu can still
-- offer "reset"; run_check() itself refuses to touch the network while
-- disabled.
local auto_check_allowed = true

do
	local saved = read_json_file(state_path)
	if saved then
		for key, value in pairs(saved) do
			if type(value) == type(state[key]) or state[key] == nil then
				state[key] = value
			end
		end
	end
	if state.disabled == true then
		auto_check_allowed = false
		mp.msg.info("update-check is disabled by saved state; automatic checks will not run")
	end
end

local local_version
do
	local info = read_json_file(version_path)
	if info and type(info.playkit_version) == "string" then
		local_version = info.playkit_version
	end
end

if not local_version then
	auto_check_allowed = false
	mp.msg.warn("update-check: playkit-version.json is missing; skipping automatic update check")
end

local function parse_version(value)
	value = tostring(value or ""):gsub("^v", "")
	local parts = {}
	for part in value:gmatch("[^.]+") do
		local number = tonumber(part)
		if not number then return nil end
		parts[#parts + 1] = number
	end
	if #parts == 0 then return nil end
	return parts
end

if local_version and not parse_version(local_version) then
	auto_check_allowed = false
	mp.msg.info("update-check: local version is not a release version; skipping automatic update check")
end

local function is_newer(candidate, current)
	local a, b = parse_version(candidate), parse_version(current)
	if not a or not b then return false end
	for index = 1, math.max(#a, #b) do
		local left, right = a[index] or 0, b[index] or 0
		if left ~= right then return left > right end
	end
	return false
end

local function valid_release_url(url)
	return type(url) == "string" and url:match("^https://github%.com/") ~= nil
end

local function show_notice()
	local text = string.format(
		"发现新的 mpv PlayKit 版本 %s（当前 %s）\n右键菜单 → 更新，可打开 Release 页面或关闭检查",
		state.latest_version,
		local_version
	)
	mp.osd_message(text, 10)
	mp.msg.info("update-check: newer release available: " .. state.latest_version)
end

local function prompt_if_newer()
	if state.disabled then return end
	if not valid_release_url(state.latest_url) then return end
	if not is_newer(state.latest_version, local_version) then return end
	if state.dismissed_version == state.latest_version then return end
	if tonumber(state.snoozed_until) and tonumber(state.snoozed_until) > os.time() then return end
	show_notice()
end

local platform = mp.get_property("platform") or "linux"

local function open_url(url)
	if not valid_release_url(url) then
		mp.osd_message("没有可用的 Release 页面地址", 3000)
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

local function make_http_commands(url)
	local timeout = tostring(math.max(3, tonumber(opts.timeout_seconds) or 12))
	return {
		{
			name = "curl",
			args = {
				"curl", "-fsSL", "--max-time", timeout,
				"-H", "Accept: application/vnd.github+json",
				"-H", "User-Agent: mpv-PlayKit-update-check",
				url,
			},
		},
		{
			name = "wget",
			args = {
				"wget", "-qO-", "--timeout=" .. timeout,
				"--header=Accept: application/vnd.github+json",
				"--header=User-Agent: mpv-PlayKit-update-check",
				url,
			},
		},
	}
end

local checking = false

local function run_check(force)
	if state.disabled or checking then return end
	if not local_version or not parse_version(local_version) then return end

	local now = os.time()
	local interval = math.max(0, tonumber(opts.check_interval_hours) or 24) * 3600
	if not force and state.last_check > 0 and (now - state.last_check) < interval then
		prompt_if_newer()
		return
	end
	if not force and tonumber(state.snoozed_until) and tonumber(state.snoozed_until) > now then
		return
	end

	checking = true
	local api_url = string.format("https://api.github.com/repos/%s/releases/latest", opts.repository)
	local commands = make_http_commands(api_url)

	local function try(index)
		if state.disabled then
			checking = false
			return
		end
		if index > #commands then
			checking = false
			state.last_check = now
			write_state()
			mp.msg.warn("update-check: no working HTTP client (curl or wget) or request failed")
			return
		end

		local attempt = commands[index]
		mp.command_native_async({
			name = "subprocess",
			args = attempt.args,
			capture_stdout = true,
			capture_stderr = true,
			playback_only = false,
		}, function(success, result)
			-- The user may have disabled the checker while curl/wget was in
			-- flight; do not launch the next HTTP client in that case.
			if state.disabled then
				checking = false
				return
			end
			if success and result and result.status == 0 and type(result.stdout) == "string"
				and #result.stdout > 0 and #result.stdout <= 256 * 1024 then
				local ok, release = pcall(utils.parse_json, result.stdout)
				if ok and type(release) == "table"
					and type(release.tag_name) == "string"
					and valid_release_url(release.html_url) then
					checking = false
					state.last_check = now
					state.latest_version = release.tag_name:gsub("^v", "")
					state.latest_url = release.html_url
					write_state()
					prompt_if_newer()
					return
				end
			end
			try(index + 1)
		end)
	end

	try(1)
end

mp.add_key_binding(nil, "check-now", function()
	run_check(true)
end)

mp.add_key_binding(nil, "open-release", function()
	if valid_release_url(state.latest_url) then
		open_url(state.latest_url)
	else
		open_url(releases_url)
	end
end)

mp.add_key_binding(nil, "snooze", function()
	state.snoozed_until = os.time() + math.max(1, tonumber(opts.snooze_hours) or 24) * 3600
	write_state()
	mp.osd_message("已推迟更新提示", 3000)
end)

mp.add_key_binding(nil, "disable", function()
	state.disabled = true
	write_state()
	mp.osd_message("已关闭 PlayKit 更新检查；删除 update-check-state.json 或使用 reset 可恢复", 5000)
end)

mp.add_key_binding(nil, "reset", function()
	state.disabled = false
	state.snoozed_until = 0
	state.dismissed_version = ""
	write_state()
	mp.osd_message("已恢复 PlayKit 更新检查", 3000)
	run_check(true)
end)

-- Delay the first check slightly so player startup is not competing with a
-- subprocess.  run_check() returns immediately after spawning curl/wget.  When
-- the saved state is disabled or the local version is unusable, do not even
-- schedule it; the menu's "reset" action can still enable checks explicitly.
if auto_check_allowed then
	mp.add_timeout(4, function()
		run_check(false)
	end)
end
