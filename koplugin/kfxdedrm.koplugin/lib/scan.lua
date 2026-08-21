--[[--
The books under `cfg.scan_dirs`, one level deep.

That depth excludes `Downloads/Items01/updates/`, the `.sdr` sidecar trees, and
any output folder written inside a scanned root.

`Scan.isEncrypted` gates every entry. The engine copies whatever it receives
into `Config.OUT_DIR`, and a DRM-free book yields a second copy of itself.

A book's `done` waits on every file `Config.OUT_DIR` should hold for it --
`Convert.outputs` included -- so a conversion that failed leaves the book listed
for another run.

`Scan.candidates` is the other half: which folders the settings menu offers,
read off the device rather than guessed.

A book's title comes from the filename. No book is opened.

A port of `native/src/scan.rs`, less its cover lookup: nothing here draws one.
]]

local lfs = require("libs/libkoreader-lfs")

local Config = require("lib.config")
local Convert = require("lib.convert")
local Engine = require("lib.engine")
local Mobi = require("lib.mobi")

local Scan = {}

local function basename(path)
    return path:match("([^/]*)$")
end

local function exists(path)
    return lfs.attributes(path, "mode") ~= nil
end

--- `<stem>.sdr/assets/voucher` beside `kfx`.
local function voucher_path(kfx)
    local dir = kfx:match("^(.*)/[^/]*$")
    if not dir then return nil end
    local stem = Engine.stemAndExtension(kfx)
    return Engine.join(dir, stem .. ".sdr/assets/voucher")
end

--- Whether `path` carries DRM.
---
--- A KFX book takes its voucher, which is the decrypt key and the mark of a
--- finished download. A MOBI-family book takes its own header.
function Scan.isEncrypted(path, format)
    if format == Engine.KFX then
        local voucher = voucher_path(path)
        return voucher ~= nil and lfs.attributes(voucher, "mode") == "file"
    end
    return Mobi.isEncrypted(path)
end

--- The format of a DRM'd book at `path` this `cfg` lists, or `nil`.
---
--- Shared by `candidate` and `count_books`, so a folder's count matches what
--- the list would actually show there.
local function listable(path, cfg)
    local name = basename(path)
    -- AppleDouble shadows carry the name of a real file on a FAT partition.
    if name:sub(1, 2) == "._" then return nil end

    local format = Engine.formatOf(path)
    if not format then return nil end

    -- Not `a and b or c`: that yields `c` whenever `b` is false, and `b` being
    -- false is the whole point of this gate.
    local wanted
    if format == Engine.KFX then
        wanted = cfg.types_kfx
    else
        wanted = cfg.types_mobi
    end
    if not wanted then return nil end

    if lfs.attributes(path, "mode") ~= "file" then return nil end
    if not Scan.isEncrypted(path, format) then return nil end
    return format
end

--- The final `_`-delimited token of `stem`, when it is `B` plus nine uppercase
--- alphanumerics.
function Scan.parseAsin(stem)
    local tok = stem:match("([^_]*)$")
    if not tok then return nil end
    local well_formed = #tok == 10
        and tok:sub(1, 1) == "B"
        and tok:match("^[A-Z0-9]+$") ~= nil
    return well_formed and tok or nil
end

--- `stem` without its trailing `_<asin>`, `_ ` restored to `: `.
function Scan.titleFromStem(stem, asin)
    local title = stem
    if asin then
        local suffix = "_" .. asin
        if title:sub(-#suffix) == suffix then
            title = title:sub(1, #title - #suffix)
        end
    end
    return (title:gsub("_ ", ": "))
end

--- One book, or `nil` for an entry that fails a gate.
local function candidate(path, cfg, targets, out_dir)
    local format = listable(path, cfg)
    if not format then return nil end

    -- `cfg.show_done` keeps or drops these.
    local done = false
    local out = Engine.outputPath(path, out_dir)
    if out and exists(out) then
        done = true
        for _, extra in ipairs(Convert.outputs(targets, out)) do
            if not exists(extra) then
                done = false
                break
            end
        end
    end
    if done and not cfg.show_done then return nil end

    local stem = Engine.stemAndExtension(path)
    local asin = Scan.parseAsin(stem)
    return {
        path = path,
        format = format,
        title = Scan.titleFromStem(stem, asin),
        asin = asin,
        size = lfs.attributes(path, "size") or 0,
        mtime = lfs.attributes(path, "modification") or 0,
        done = done,
    }
end

--- `candidate` over one directory's entries.
local function scan_root(root, cfg, targets, out_dir)
    local found = {}
    -- Each root is optional on any one device.
    if lfs.attributes(root, "mode") ~= "directory" then return found end
    local ok, iter, dir_obj = pcall(lfs.dir, root)
    if not ok then return found end
    for name in iter, dir_obj do
        if name ~= "." and name ~= ".." then
            local book = candidate(Engine.join(root, name), cfg, targets, out_dir)
            if book then table.insert(found, book) end
        end
    end
    return found
end

--- Books across `roots`, outputs judged against `out_dir`.
---
--- `roots` order, `mtime` descending within each. Paths break a tie, which
--- `lfs.dir` alone would leave to the filesystem.
function Scan.scanIn(roots, cfg, targets, out_dir)
    local out = {}
    for _, root in ipairs(roots) do
        local found = scan_root(root, cfg, targets, out_dir)
        table.sort(found, function(a, b)
            if a.mtime ~= b.mtime then return a.mtime > b.mtime end
            return a.path < b.path
        end)
        for _, book in ipairs(found) do
            table.insert(out, book)
        end
    end
    return out
end

--- `Scan.scanIn` over `cfg.scan_dirs` and `Config.OUT_DIR`.
function Scan.scan(cfg, targets)
    return Scan.scanIn(cfg.scan_dirs, cfg, targets, Config.OUT_DIR)
end

--- How far under `Config.DOCUMENTS_DIR` `Scan.candidates` looks.
---
--- Two levels reaches `Downloads/Items01`, which is where current firmware
--- puts purchases, without walking a library's worth of sidecars.
local PROBE_DEPTH = 2

--- `listable` entries directly inside `dir`.
local function count_books(dir, cfg)
    local n = 0
    if lfs.attributes(dir, "mode") ~= "directory" then return n end
    local ok, iter, dir_obj = pcall(lfs.dir, dir)
    if not ok then return n end
    for name in iter, dir_obj do
        if name ~= "." and name ~= ".." and listable(Engine.join(dir, name), cfg) then
            n = n + 1
        end
    end
    return n
end

--- `dir`, then its subdirectories down to `depth`.
---
--- `.sdr` sidecars are skipped: every KFX book has one, so a library's worth of
--- them is most of what `Config.DOCUMENTS_DIR` holds and none of them is a
--- folder anyone scans.
local function collect_dirs(dir, depth, out)
    if lfs.attributes(dir, "mode") ~= "directory" then return end
    table.insert(out, dir)
    if depth == 0 then return end
    local ok, iter, dir_obj = pcall(lfs.dir, dir)
    if not ok then return end
    local children = {}
    for name in iter, dir_obj do
        if name ~= "." and name ~= ".." then
            local path = Engine.join(dir, name)
            local _stem, ext = Engine.stemAndExtension(path)
            local is_sidecar = ext ~= nil and ext:lower() == "sdr"
            if not is_sidecar and lfs.attributes(path, "mode") == "directory" then
                table.insert(children, path)
            end
        end
    end
    table.sort(children)
    for _, child in ipairs(children) do
        collect_dirs(child, depth - 1, out)
    end
end

--- The label a folder takes in the settings menu: the path relative to
--- `Config.DOCUMENTS_DIR`, or that folder's own name.
---
--- A folder outside it keeps its leading `/`, which is what tells the two apart
--- on a list of short names.
function Scan.folderLabel(dir)
    local documents = Config.DOCUMENTS_DIR
    if dir == documents then
        return basename(documents)
    end
    if dir:sub(1, #documents + 1) == documents .. "/" then
        return dir:sub(#documents + 2)
    end
    return dir
end

--- `Scan.candidatesIn` over `Config.DOCUMENTS_DIR`.
function Scan.candidates(cfg)
    return Scan.candidatesIn(Config.DOCUMENTS_DIR, cfg)
end

--- Folders that hold a DRM'd book, plus every folder already selected.
---
--- Read off the device rather than guessed: which folder a firmware downloads
--- into has moved before, and a sideload folder is whatever its owner named it.
--- A selected folder stays on the list at zero books, or deselecting it would
--- mean deselecting a row that is no longer drawn.
function Scan.candidatesIn(root, cfg)
    local dirs = {}
    collect_dirs(root, PROBE_DEPTH, dirs)
    -- A selected folder may sit outside `root` entirely, having been written
    -- into the file by hand.
    for _, dir in ipairs(cfg.scan_dirs) do
        local seen = false
        for _, known in ipairs(dirs) do
            if known == dir then
                seen = true
                break
            end
        end
        if not seen then table.insert(dirs, dir) end
    end

    local out = {}
    for _, dir in ipairs(dirs) do
        local books = count_books(dir, cfg)
        local selected = false
        for _, chosen in ipairs(cfg.scan_dirs) do
            if chosen == dir then
                selected = true
                break
            end
        end
        if books > 0 or selected then
            table.insert(out, { dir = dir, books = books })
        end
    end
    return out
end

--- Books with no output yet, the ones a batch run has work for.
function Scan.pending(books)
    local n = 0
    for _, book in ipairs(books) do
        if not book.done then n = n + 1 end
    end
    return n
end

return Scan
