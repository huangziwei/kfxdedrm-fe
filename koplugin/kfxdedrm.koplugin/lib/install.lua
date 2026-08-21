--[[--
Fetching the engine and the bokai add-on from their own GitHub releases.

Neither ships with this plugin -- one is someone else's project and both are
several megabytes of ARM binary -- so what is offered here is the download the
README otherwise asks for by hand.

Two things about those repositories decide the shape of this:

- **`/releases/latest` is the wrong endpoint.** Every DeDRM_tools release that
  carries `kfxdedrmmobi.zip` is marked a prerelease, which that endpoint skips,
  and a sidle tag is published before its assets finish uploading. `pickRelease`
  walks the release list instead and takes the newest one that actually holds a
  matching asset.
- **The two zips have different roots** -- `kfxdedrm/…` against
  `extensions/bokai/…` -- and neither matches the folder they install into. So
  no path inside the archive is hardcoded: `prefixFor` finds the one entry
  ending in the source's `marker` and everything under that entry's prefix is
  what gets extracted.

An install is staged beside its destination and has to prove itself -- the
engine by `Engine.locateIn`, bokai by `Convert.locateAt` -- before it replaces
what is already there. A download that arrives corrupt, or built for another
ABI, therefore costs nothing.
]]

local lfs = require("libs/libkoreader-lfs")
local logger = require("logger")
local util = require("util")

local Convert = require("lib.convert")
local Engine = require("lib.engine")

local Install = {}

--- Releases to look through, newest first. Well past the depth at which the
--- engine last shipped its asset.
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
        --- The version rides the filename, so no one name belongs here.
        asset = "^bokai%-.*%-kindle%.zip$",
        --- Names the archive's root: `extensions/bokai/bin/bokai`.
        marker = "bin/bokai",
        dest = Convert.EXTENSION_DIR,
        verify = function(dir)
            return Convert.locateAt(dir .. "/bin/bokai") ~= nil
        end,
    },
}

function Install.source(key)
    for _, source in ipairs(Install.SOURCES) do
        if source.key == key then return source end
    end
    return nil
end

--------------------------------------------------------------------------------
-- Pure: what to fetch, and what to pull out of the archive
--------------------------------------------------------------------------------

--- The newest release in `releases` carrying an asset `source` names.
---
--- Returns the tag, the asset's download URL and its name, plus the URL of a
--- `.sha256` sidecar when the release publishes one.
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
                return release.tag_name, found.browser_download_url, found.name, sha
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
            -- The sink writes what arrives; a compressed body would not be it.
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

--- A GET straight to `path`. The partial file is removed on any failure, so a
--- half-download is never left looking like an archive.
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
--- `ffi/sha2` would do this without one, but it is pure Lua over a file of
--- several megabytes and this runs on a Kindle. No tool means no check, which
--- the caller reports rather than treats as a failure.
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
--- libarchive creates the leading directories and refuses `..` in a path, but
--- does not carry the mode across, so nothing under `bin/` comes out
--- executable -- see the chmod in `Install.run`.
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

--- Where a download is staged. Beside the destination, on the same partition,
--- so the swap that follows is a rename.
local function staging_of(dest)
    return dest .. ".new"
end

--- The release `source` would install, or `nil` and a message.
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

    local tag, asset_url, asset_name, sha_url = Install.pickRelease(releases, source)
    if not tag then
        return nil, "no release carries " .. source.name
    end
    return { tag = tag, url = asset_url, name = asset_name, sha = sha_url }
end

--- Download, unpack, prove and swap in. `progress` takes one line at a time.
---
--- Returns the tag installed, or `nil` and a message. Nothing already in place
--- is touched until a staged copy has run on this device.
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
        -- Put back whatever was working before failing.
        if lfs.attributes(previous, "mode") == "directory" then
            os.execute("mv " .. util.shell_escape({ previous }) .. " " .. util.shell_escape({ source.dest }))
        end
        rm_rf(staging)
        return nil, "cannot move the new copy into " .. source.dest
    end
    rm_rf(previous)

    logger.info("kfxdedrm: installed", source.name, release.tag, "into", source.dest)
    return release.tag
end

--------------------------------------------------------------------------------
-- Which release is on the device
--------------------------------------------------------------------------------

--- Where both frontends record what they installed.
---
--- Shared with `native/`, which reads and writes the same file in the same
--- format, so one device carries one record and neither frontend fetches what
--- the other already has.
---
--- Its own file rather than a couple of keys in the settings `lib/config`
--- shares: that format is fixed on both sides and a key one frontend does not
--- know is a key its next save drops. This one has no such constraint.
---
--- Neither binary reports its own version, so an install neither frontend made
--- reads as unknown.
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
