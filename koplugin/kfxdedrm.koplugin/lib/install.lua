--[[--
`Install.run` fetches each `Install.SOURCES` entry from GitHub into its own
extension folder, and `Install.appSource` points it at this plugin. A port of
`native/src/install/mod.rs` and `native/src/install/selfupdate.rs`.
]]

local lfs = require("libs/libkoreader-lfs")
local logger = require("logger")
local util = require("util")

local Convert = require("lib.convert")
local Engine = require("lib.engine")

local Install = {}

--- Releases to look through, newest first.
local RELEASES_PER_PAGE = 30

--- A 5 MB binary over a Kindle's wifi outlasts `socketutil`'s file default.
local DOWNLOAD_TOTAL_TIMEOUT = 300

--- What can be installed. `verify` is what a staged copy has to pass.
Install.SOURCES = {
    {
        key = "engine",
        name = "kfxdedrm",
        repo = "Satsuoni/DeDRM_tools",
        --- `kfxdedrm.zip` and `kfxdedrm_kual.zip` are older, KFX-only assets.
        asset = "^kfxdedrmmobi%.zip$",
        --- `kfxdedrmmobi.zip` carries no version; the tag is the whole of it.
        version = function(_asset, tag) return tag end,
        --- Names the archive's root: `kfxdedrm/bin/kfxdedrmhf_c11`.
        marker = "bin/kfxdedrmhf_c11",
        dest = Engine.EXTENSION_DIR,
        verify = function(dir)
            return Engine.locateIn(dir .. "/bin") ~= nil
        end,
    },
    {
        key = "bokai",
        name = "bokai",
        repo = "huangziwei/sidle",
        --- The version rides the filename.
        asset = "^bokai%-.*%-kindle%.zip$",
        --- A sidle tag names sidle. bokai's own version rides the filename.
        version = function(asset, tag)
            return Install.bokaiVersion(asset) or tag
        end,
        --- Names the archive's root: `extensions/bokai/bin/bokai`.
        marker = "bin/bokai",
        dest = Convert.EXTENSION_DIR,
        verify = function(dir)
            return Convert.locateIn(dir .. "/bin") ~= nil
        end,
    },
}

function Install.source(key)
    for _, source in ipairs(Install.SOURCES) do
        if source.key == key then return source end
    end
    return nil
end

--- Where both frontends are published, and what `aboutText` names for anyone
--- doing it by hand.
Install.RELEASES_URL = "github.com/huangziwei/kfxdedrm-fe/releases"

--- This plugin, as a source `Install.run` installs like any other. `dest` is
--- the plugin's own folder, and `Install.metaVersion` reports the version
--- `Install.RECORD_PATH` keeps no line for.
function Install.appSource(dest)
    return {
        key = "koplugin",
        name = "kfxdedrm-koplugin",
        repo = "huangziwei/kfxdedrm-fe",
        --- The version rides the filename.
        asset = "^kfxdedrm%-koplugin%-.+%.zip$",
        version = function(asset, tag)
            return asset:match("^kfxdedrm%-koplugin%-(.+)%.zip$") or tag
        end,
        --- Names the archive's root: `kfxdedrm.koplugin/_meta.lua`. The
        --- LICENSE beside it falls outside and is skipped.
        marker = "_meta.lua",
        dest = dest,
        verify = function(dir)
            return Install.metaVersion(dir .. "/_meta.lua") ~= nil
                and lfs.attributes(dir .. "/main.lua", "mode") == "file"
        end,
    }
end

--------------------------------------------------------------------------------
-- Pure: what to fetch, and what to pull out of the archive
--------------------------------------------------------------------------------

--- The version in `bokai-<version>-kindle.zip`.
function Install.bokaiVersion(asset)
    local version = asset:match("^bokai%-(.+)%-kindle%.zip$")
    if version == "" then return nil end
    return version
end

--- The `version` line of a `_meta.lua`, read as text.
---
--- `PluginLoader` copies the same field onto every module it loads; this
--- answers for a staged download, and for a copy required some other way.
function Install.metaVersion(path)
    local file = io.open(path, "r")
    if not file then return nil end
    local text = file:read("*all")
    file:close()
    return text and text:match('version%s*=%s*"([^"]*)"')
end

--- The widest a version component may be, matching the `u32` the Rust port
--- reads one into.
local COMPONENT_MAX = 4294967295

--- `version` as its numbers and whatever followed them: `v0.5.0-rc1` reads as
--- `{ 0, 5, 0 }, "-rc1"`. `nil` when it opens with no number at all.
local function parts(version)
    local body = tostring(version or ""):gsub("^v", "")
    local numbers, suffix = body:match("^([%d%.]*)(.*)$")
    local out = {}
    for piece in numbers:gmatch("[^%.]+") do
        local n = tonumber(piece)
        if not n or n ~= math.floor(n) or n > COMPONENT_MAX then return nil end
        out[#out + 1] = n
    end
    if #out == 0 then return nil end
    return out, suffix
end

--- Whether `offered` names a later release than `installed`: dot-separated
--- numbers, an optional leading `v`, and a `-rc1` suffix sorting before the
--- same numbers without one. `install::selfupdate::is_newer` reads them alike.
function Install.isNewer(offered, installed)
    local a, a_suffix = parts(offered)
    local b, b_suffix = parts(installed)
    if not a or not b then return false end
    for i = 1, math.max(#a, #b) do
        local x, y = a[i] or 0, b[i] or 0
        if x ~= y then return x > y end
    end
    return a_suffix == "" and b_suffix ~= ""
end

--- The newest release in `releases` carrying an asset `source` names.
---
--- Returns `source.version`, the asset's download URL and its name, plus the
--- URL of a `.sha256` sidecar when the release publishes one.
function Install.pickRelease(releases, source)
    for _, release in ipairs(releases or {}) do
        if not release.draft then
            local found, sha
            for _, asset in ipairs(release.assets or {}) do
                if asset.name and asset.name:match(source.asset) then
                    found = asset
                end
            end
            if found then
                for _, asset in ipairs(release.assets) do
                    if asset.name == found.name .. ".sha256" then
                        sha = asset.browser_download_url
                    end
                end
                return source.version(found.name, release.tag_name),
                    found.browser_download_url, found.name, sha
            end
        end
    end
    return nil
end

--- Everything in `paths` sits under one folder; this is that folder.
---
--- The entry ending in `marker` is what names it: whatever precedes the marker
--- is the prefix, `""` for an archive that unpacks into its own root.
function Install.prefixFor(paths, marker)
    for _, path in ipairs(paths) do
        if path == marker then
            return ""
        end
        local prefix = path:match("^(.*/)" .. marker:gsub("%p", "%%%0") .. "$")
        if prefix then return prefix end
    end
    return nil
end

--- The `sha256sum`-style line for `name`, or the whole file when it carries a
--- bare digest.
function Install.digestFrom(text, name)
    for line in (text or ""):gmatch("[^\r\n]+") do
        local digest, named = line:match("^(%x+)%s+%*?(.+)$")
        if digest and #digest == 64 and (named == name or named:match("[^/]*$") == name) then
            return digest:lower()
        end
        local bare = line:match("^(%x+)%s*$")
        if bare and #bare == 64 then return bare:lower() end
    end
    return nil
end

--------------------------------------------------------------------------------
-- The network and the disk
--------------------------------------------------------------------------------

--- A GET into memory. Returns the body, or `nil` and a message.
local function fetch(url, accept)
    local http = require("socket.http")
    local ltn12 = require("ltn12")
    local socket = require("socket")
    local socketutil = require("socketutil")

    local body = {}
    socketutil:set_timeout(socketutil.LARGE_BLOCK_TIMEOUT, socketutil.LARGE_TOTAL_TIMEOUT)
    local code, _headers, status = socket.skip(1, http.request{
        url = url,
        headers = {
            ["Accept"] = accept or "*/*",
            -- The sink writes what arrives, uncompressed.
            ["Accept-Encoding"] = "identity",
        },
        sink = ltn12.sink.table(body),
    })
    socketutil:reset_timeout()

    if code ~= 200 then
        return nil, tostring(status or code or "no reply")
    end
    return table.concat(body)
end

--- A GET straight to `path`. The partial file is removed on any failure.
local function download(url, path)
    local http = require("socket.http")
    local ltn12 = require("ltn12")
    local socket = require("socket")
    local socketutil = require("socketutil")

    local sink = io.open(path, "w")
    if not sink then
        return nil, "cannot write " .. path
    end

    socketutil:set_timeout(socketutil.FILE_BLOCK_TIMEOUT, DOWNLOAD_TOTAL_TIMEOUT)
    local code, _headers, status = socket.skip(1, http.request{
        url = url,
        headers = { ["Accept-Encoding"] = "identity" },
        sink = ltn12.sink.file(sink),
    })
    socketutil:reset_timeout()

    if code ~= 200 then
        os.remove(path)
        return nil, tostring(status or code or "no reply")
    end
    return true
end

--- SHA-256 of `path` through whichever tool the device has.
---
--- No tool means no digest, which the caller reports.
local function digest_of(path)
    for _, tool in ipairs({ "sha256sum", "shasum -a 256" }) do
        local pipe = io.popen(tool .. " " .. util.shell_escape({ path }) .. " 2>/dev/null")
        local line = pipe and pipe:read("*line")
        if pipe then pipe:close() end
        local digest = line and line:match("^(%x+)")
        if digest and #digest == 64 then return digest:lower() end
    end
    return nil
end

local function rm_rf(path)
    os.execute("rm -rf " .. util.shell_escape({ path }))
end

--- Every entry under the archive's own root, into `dest`.
---
--- libarchive creates leading directories and refuses `..`. It drops the
--- mode; `Install.run` chmods `bin/`.
function Install.unpack(zip, marker, dest)
    local Archiver = require("ffi/archiver")

    local reader = Archiver.Reader:new()
    if not reader:open(zip) then
        return nil, reader.err or "not an archive"
    end

    local paths, modes = {}, {}
    for entry in reader:iterate() do
        paths[#paths + 1] = entry.path
        modes[entry.path] = entry.mode
    end

    local prefix = Install.prefixFor(paths, marker)
    if not prefix then
        reader:close()
        return nil, "no " .. marker .. " inside"
    end

    local written = 0
    for _, path in ipairs(paths) do
        if modes[path] == "file" and path:sub(1, #prefix) == prefix then
            local rest = path:sub(#prefix + 1)
            if rest ~= "" then
                local ok = reader:extractToPath(path, dest .. "/" .. rest)
                if not ok then
                    reader:close()
                    return nil, "cannot unpack " .. rest
                end
                written = written + 1
            end
        end
    end
    reader:close()

    if written == 0 then
        return nil, "nothing to unpack"
    end
    return written
end

--------------------------------------------------------------------------------
-- The whole flow
--------------------------------------------------------------------------------

--- Where a download is staged, beside the destination on one partition.
local function staging_of(dest)
    return dest .. ".new"
end

--- The release `source` installs, or `nil` and a message.
function Install.available(source)
    local rapidjson = require("rapidjson")

    local url = "https://api.github.com/repos/" .. source.repo
        .. "/releases?per_page=" .. RELEASES_PER_PAGE
    local body, err = fetch(url, "application/vnd.github+json")
    if not body then
        return nil, err
    end

    local ok, releases = pcall(rapidjson.decode, body)
    if not ok or type(releases) ~= "table" then
        return nil, "unreadable reply"
    end

    local version, asset_url, asset_name, sha_url = Install.pickRelease(releases, source)
    if not version then
        return nil, "no release carries " .. source.name
    end
    return { version = version, url = asset_url, name = asset_name, sha = sha_url }
end

--- Download, unpack, prove and swap in. `progress` takes one line at a time.
--- Returns the version installed, or `nil` and a message. `source.dest` is
--- untouched until a staged copy has run on this device.
function Install.run(source, release, progress)
    local function say(text)
        if progress then progress(text) end
    end

    local staging = staging_of(source.dest)
    local zip = staging .. ".zip"
    rm_rf(staging)
    os.remove(zip)

    say("Downloading " .. release.name .. "…")
    local ok, err = download(release.url, zip)
    if not ok then
        return nil, "download failed: " .. tostring(err)
    end

    if release.sha then
        local text = fetch(release.sha)
        local want = text and Install.digestFrom(text, release.name)
        local got = want and digest_of(zip)
        if want and got and want ~= got then
            os.remove(zip)
            return nil, "the download does not match its published checksum"
        end
        logger.info("kfxdedrm: checksum", want and (got and "matched" or "not checked here") or "not published")
    end

    say("Unpacking…")
    local written
    written, err = Install.unpack(zip, source.marker, staging)
    os.remove(zip)
    if not written then
        rm_rf(staging)
        return nil, "unpack failed: " .. tostring(err)
    end

    -- libarchive drops the mode; without this nothing under bin/ can be run.
    os.execute("chmod +x " .. util.shell_escape({ staging .. "/bin" }) .. "/* 2>/dev/null")

    say("Checking it runs here…")
    if not source.verify(staging) then
        rm_rf(staging)
        return nil, "the downloaded build does not run on this Kindle"
    end

    local previous = source.dest .. ".old"
    rm_rf(previous)
    if lfs.attributes(source.dest, "mode") == "directory" then
        os.execute("mv " .. util.shell_escape({ source.dest }) .. " " .. util.shell_escape({ previous }))
    end
    local moved = os.execute("mv " .. util.shell_escape({ staging }) .. " " .. util.shell_escape({ source.dest })) == 0
    if not moved then
        -- The previous copy back into place.
        if lfs.attributes(previous, "mode") == "directory" then
            os.execute("mv " .. util.shell_escape({ previous }) .. " " .. util.shell_escape({ source.dest }))
        end
        rm_rf(staging)
        return nil, "cannot move the new copy into " .. source.dest
    end
    rm_rf(previous)

    logger.info("kfxdedrm: installed", source.name, release.version, "into", source.dest)
    return release.version
end

--------------------------------------------------------------------------------
-- Which release is on the device
--------------------------------------------------------------------------------

--- Where both frontends record what they installed.
---
--- `native/src/install/record.rs` renders the same bytes.
Install.RECORD_PATH = "/mnt/us/extensions/kfxdedrm-fe/installs.txt"

--- The file's header, matching `install::record::Record::render` on the Rust
--- side byte for byte.
local RECORD_HEADER = table.concat({
    "# Which release of each add-on is installed. Both frontends write this file,",
    "# the standalone kfxdedrm-fe app and the KOReader plugin, so neither fetches",
    "# what the other already has. Delete a line to fetch that one again.",
    "",
}, "\n")

--- `key = value` lines. A file that is not there is an empty record, and a
--- line without both halves is no record of anything.
function Install.record(path)
    local tags = {}
    local file = io.open(path or Install.RECORD_PATH, "r")
    if not file then return tags end
    for line in file:lines() do
        line = line:match("^%s*(.-)%s*$")
        if line ~= "" and line:sub(1, 1) ~= "#" then
            local key, value = line:match("^(.-)%s*=%s*(.-)$")
            if key and key ~= "" and value ~= "" then
                tags[key:match("^%s*(.-)%s*$")] = value
            end
        end
    end
    file:close()
    return tags
end

--- `tags` back out, keys in order, under `RECORD_HEADER`.
function Install.renderRecord(tags)
    local keys = {}
    for key in pairs(tags) do keys[#keys + 1] = key end
    table.sort(keys)

    local out = { RECORD_HEADER }
    for _, key in ipairs(keys) do
        out[#out + 1] = key .. " = " .. tags[key] .. "\n"
    end
    return table.concat(out)
end

function Install.installedTag(key, path)
    return Install.record(path)[key]
end

function Install.rememberTag(key, tag, path)
    path = path or Install.RECORD_PATH
    local tags = Install.record(path)
    tags[key] = tag
    local file = io.open(path, "w")
    if not file then
        logger.warn("kfxdedrm: cannot write", path)
        return
    end
    file:write(Install.renderRecord(tags))
    file:close()
end

return Install
