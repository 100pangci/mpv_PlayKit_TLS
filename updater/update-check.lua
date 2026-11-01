-- mpv_PlayKit update checker
--
-- This script *detects* a newer PlayKit release, shows a notice, and can ask
-- the bundled Rust helper to perform a verified, transactional update.  The
-- Lua script itself never downloads or replaces the player: all file changes
-- happen in `updater/playkit-updater`.  Set `enabled=no` in
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
	install_enabled = true,
	auto_repair = true,
	auto_repair_relaunch = "auto",
	unmanaged = "capture",
	relaunch = "auto",
	wait_timeout = 600,
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
-- playkit-version.json lives at the installation root, one level above
-- portable_config.  Keep the old location as a fallback for installations
-- created by an older package layout.
local version_paths = {
	mp.command_native({ "expand-path", "~~/../playkit-version.json" }),
	mp.command_native({ "expand-path", "~~/playkit-version.json" }),
}
-- The API check cannot see releases of a private repository without a token,
-- but the user's browser session can.  Keep a release-page fallback so the
-- menu item always opens something useful.
local releases_url = string.format("https://github.com/%s/releases/latest", opts.repository)
local platform = mp.get_property("platform") or "linux"
local root_path = mp.command_native({ "expand-path", "~~/.." })
local helper_name = platform == "windows" and "playkit-updater.exe" or "playkit-updater"
local helper_path = mp.command_native({ "expand-path", "~~/../updater/" .. helper_name })
local status_path = mp.command_native({ "expand-path", "~~/.playkit/update-status.json" })

local state = {
	schema = 1,
	last_check = 0,
	latest_version = "",
	latest_tag = "",
	latest_url = "",
	snoozed_until = 0,
	dismissed_version = "",
	disabled = false,
	last_repaired_version = "",
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
	for _, path in ipairs(version_paths) do
		local info = read_json_file(path)
		if info and type(info.playkit_version) == "string" then
			local_version = info.playkit_version
			break
		end
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
		"发现新的 mpv PlayKit 版本 %s（当前 %s）\n右键菜单 → 更新，可下载安装、打开 Release 页面或关闭检查",
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
					state.latest_tag = release.tag_name
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

local function file_exists(path)
	if type(path) ~= "string" then return false end
	local file = io.open(path, "rb")
	if not file then return false end
	file:close()
	return true
end

local function current_pid()
	if utils.getpid then
		local pid = utils.getpid()
		if type(pid) == "number" and pid > 0 then return tostring(pid) end
	end
	return "0"
end

local function policy(value, allowed, fallback)
	value = tostring(value or ""):lower()
	for _, item in ipairs(allowed) do
		if value == item then return value end
	end
	return fallback
end

local status_timer = nil
local last_status_key = nil

local function stop_status_polling()
	if status_timer then
		status_timer:kill()
		status_timer = nil
	end
end

local function poll_update_status()
	local status = read_json_file(status_path, 128 * 1024)
	if not status or type(status.state) ~= "string" then return end
	local key = status.state .. "|" .. tostring(status.message) .. "|" .. tostring(status.progress)
	if key == last_status_key then return end
	last_status_key = key
	local message = tostring(status.message or "正在更新…")
	if status.state == "installed" or status.state == "rolled-back"
		or status.state == "repaired" or status.state == "captured"
		or status.state == "up-to-date" or status.state == "relaunched" then
		stop_status_polling()
		mp.osd_message(message, 8000)
		return
	end
	if status.state == "blocked" or status.state == "failed" then
		stop_status_polling()
		mp.msg.warn(
			"update-check: " .. tostring(status.state) .. ": " .. message
			.. " (" .. tostring(status.detail or "") .. ")"
		)
		mp.osd_message(
			"更新未完成：" .. message
			.. "\n详情见 portable_config/.playkit/updater.log",
			12000
		)
		return
	end
	if status.progress then
		message = string.format("%s（%d%%）", message, tonumber(status.progress) or 0)
	end
	mp.osd_message(message, 5000)
end

local function start_status_polling()
	os.remove(status_path)
	last_status_key = nil
	if not status_timer then
		status_timer = mp.add_periodic_timer(1, poll_update_status)
	end
end

local function run_helper(args, on_success)
	mp.command_native_async({
		name = "subprocess",
		args = args,
		capture_stdout = true,
		capture_stderr = true,
		playback_only = false,
	}, function(success, result)
		if success and result and result.status == 0 then
			if on_success then on_success() end
			return
		end
		stop_status_polling()
		local detail = ""
		if result and type(result.stderr) == "string" and #result.stderr > 0 then
			detail = "\n" .. result.stderr:sub(1, 300)
		end
		mp.msg.warn("update-check: updater failed to start: " .. tostring(detail))
		mp.osd_message("无法启动更新器，请查看 portable_config/.playkit/updater.log" .. detail, 10000)
	end)
end

local function updater_available()
	if truthy_disabled(opts.install_enabled) then
		mp.osd_message("播放器内安装功能已在 update-check.conf 中关闭", 5000)
		return false
	end
	if state.disabled then
		mp.osd_message("更新检查已关闭；请先在菜单中恢复更新检查", 5000)
		return false
	end
	if not file_exists(helper_path) then
		mp.osd_message(
			"找不到更新器：updater/" .. helper_name .. "\n请从 Release 页面手动下载完整安装包",
			10000
		)
		return false
	end
	if type(root_path) ~= "string" then
		mp.osd_message("无法定位 PlayKit 安装目录，已停止更新", 8000)
		return false
	end
	return true
end

local function relaunch_policy()
	return policy(opts.relaunch, { "auto", "yes", "no" }, "auto")
end

local function wait_timeout()
	local value = math.floor(tonumber(opts.wait_timeout) or 600)
	if value < 1 then value = 1 end
	if value > 3600 then value = 3600 end
	return tostring(value)
end

local function install_now()
	if not updater_available() then return end
	if not local_version or not parse_version(local_version) then
		mp.osd_message("无法识别当前 PlayKit 版本，不能自动安装", 5000)
		return
	end
	if not state.latest_version or not is_newer(state.latest_version, local_version) then
		mp.osd_message("当前已是最新版本；如需检查请先执行“立即检查更新”", 5000)
		return
	end
	local tag = state.latest_tag
	if type(tag) ~= "string" or tag == "" then tag = "v" .. state.latest_version end
	if not tag:match("^v?[%w_.%-]+$") then
		mp.osd_message("Release tag 无效，已停止安装", 5000)
		return
	end
	start_status_polling()
	mp.osd_message("正在启动更新器…", 3000)
	run_helper({
		helper_path,
		"install",
		"--root", root_path,
		"--repository", opts.repository,
		"--tag", tag,
		"--wait-pid", current_pid(),
		"--wait-timeout", wait_timeout(),
		"--unmanaged", policy(opts.unmanaged, { "capture", "discard", "abort" }, "capture"),
		"--relaunch", relaunch_policy(),
		"--quiet",
		"--detach",
	})
end

local function capture_now()
	if not updater_available() then return end
	if not local_version or not parse_version(local_version) then
		mp.osd_message("无法识别当前 PlayKit 版本，不能保存配置修改", 5000)
		return
	end
	start_status_polling()
	mp.osd_message("正在保存当前配置修改…", 3000)
	run_helper({
		helper_path,
		"capture",
		"--root", root_path,
		"--quiet",
	})
end

local function rollback_now()
	if not updater_available() then return end
	start_status_polling()
	mp.osd_message("正在启动回滚…", 3000)
	run_helper({
		helper_path,
		"rollback",
		"--root", root_path,
		"--wait-pid", current_pid(),
		"--relaunch", relaunch_policy(),
		"--quiet",
		"--detach",
	})
end

local function repair_now()
	if not updater_available() then return end
	start_status_polling()
	mp.osd_message("正在按当前版本重新应用配置 patch…", 3000)
	run_helper({
		helper_path,
		"repair",
		"--root", root_path,
		"--quiet",
	}, function()
		state.last_repaired_version = local_version
		write_state()
	end)
end

mp.add_key_binding(nil, "check-now", function()
	run_check(true)
end)

mp.add_key_binding(nil, "install-now", install_now)
mp.add_key_binding(nil, "capture-now", capture_now)
mp.add_key_binding(nil, "rollback", rollback_now)
mp.add_key_binding(nil, "repair-now", repair_now)

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

-- Reapply the user layer automatically when patch files (or user scripts,
-- fonts and script options) changed since the last install, repair or capture.
-- This is deliberately not part of run_check(): it performs no network request
-- and is disabled together with the updater.
local function auto_relaunch_allowed()
	local mode = tostring(opts.auto_repair_relaunch or "auto"):lower()
	if mode == "no" or mode == "false" or mode == "0" then return false end
	if mode == "yes" or mode == "true" or mode == "1" then return true end
	if mode ~= "auto" then return false end
	-- Restarting while media is loaded would lose the current playback; leave
	-- that case to the user and only restart an idle startup.
	local path = mp.get_property("path")
	if type(path) == "string" and path ~= "" then return false end
	local count = tonumber(mp.get_property("playlist-count")) or 0
	return count <= 0
end

local function start_relaunch()
	local pid = current_pid()
	if pid == "0" then
		mp.osd_message("配置 patch 已重新应用，但无法确定播放器进程；请手动重启播放器", 10000)
		return
	end
	mp.command_native_async({
		name = "subprocess",
		args = {
			helper_path,
			"relaunch",
			"--root", root_path,
			"--wait-pid", pid,
			"--wait-timeout", wait_timeout(),
			"--quiet",
			"--detach",
		},
		capture_stdout = true,
		capture_stderr = true,
		playback_only = false,
	}, function(success, result)
		if not (success and result and result.status == 0) then
			local detail = ""
			if result and type(result.stderr) == "string" and #result.stderr > 0 then
				detail = "：" .. result.stderr:sub(1, 200)
			end
			mp.msg.warn("update-check: could not start relaunch helper" .. detail)
			mp.osd_message("配置 patch 已重新应用，但无法自动重启；请手动重启播放器", 10000)
			return
		end
		mp.osd_message("配置 patch 已重新应用，正在重启播放器…", 5000)
		mp.add_timeout(0.8, function()
			mp.commandv("quit")
		end)
	end)
end

local function mark_repaired()
	if state.last_repaired_version ~= local_version then
		state.last_repaired_version = local_version
		write_state()
	end
end

local function finish_auto_repair(status)
	mark_repaired()
	if status == 0 then
		if auto_relaunch_allowed() then
			start_relaunch()
		else
			mp.osd_message("配置 patch 已重新应用；重启播放器后生效", 8000)
		end
	elseif status == 4 then
		-- Repair ran, but the generated configuration is byte-identical (for
		-- example the first fingerprint initialization after installation).
		mp.msg.info("update-check: user layer repaired without configuration changes")
	end
end

local function auto_repair_allowed()
	if truthy_disabled(opts.auto_repair) or truthy_disabled(opts.install_enabled) then
		return false
	end
	if state.disabled then return false end
	if type(local_version) ~= "string" or local_version == "" then return false end
	if type(root_path) ~= "string" or not file_exists(helper_path) then return false end
	return true
end

if auto_repair_allowed() then
	mp.add_timeout(6, function()
		if not auto_repair_allowed() then return end
		mp.msg.info("update-check: checking whether the user layer needs repair")
		mp.command_native_async({
			name = "subprocess",
			args = { helper_path, "repair-auto", "--root", root_path, "--quiet" },
			capture_stdout = true,
			capture_stderr = true,
			playback_only = false,
		}, function(success, result)
			local status = nil
			if success and result then status = tonumber(result.status) end
			-- repair-auto exits 0 (changed), 3 (nothing to do) or 4
			-- (repaired without changing generated files).  Exit 64 means the
			-- installed helper predates this command; skip silently rather
			-- than running a full repair on every startup.
			if status == 64 then
				mp.msg.info("update-check: installed helper does not support automatic repair")
				return
			end
			if status == 3 then
				mark_repaired()
				return
			end
			if status == 0 or status == 4 then
				finish_auto_repair(status)
				return
			end
			local detail = ""
			if result and type(result.stderr) == "string" and #result.stderr > 0 then
				detail = "：" .. result.stderr:sub(1, 200)
			end
			mp.msg.warn("update-check: automatic repair failed" .. detail)
			mp.osd_message("用户配置自动修复失败，请查看 portable_config/.playkit/updater.log", 10000)
		end)
	end)
end

-- Delay the first check slightly so player startup is not competing with a
-- subprocess.  run_check() returns immediately after spawning curl/wget.  When
-- the saved state is disabled or the local version is unusable, do not even
-- schedule it; the menu's "reset" action can still enable checks explicitly.
if auto_check_allowed then
	mp.add_timeout(4, function()
		run_check(false)
	end)
end
