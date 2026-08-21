--[[--
`Engine.locate` the kfxdedrm engine under `Engine.BIN_DIR`,
`Engine.decryptCommand` for the shell line that runs it. Nothing in this
plugin decrypts anything.

The engine's command surface:

| invocation | effect |
|:--|:--|
| *(no args)* | decrypt everything under `documents/` |
| `test` | exits 0 if this build runs on this device |
| `dedrm <book> [outdir]` | decrypt one book |
| `dedrm_all [scandir] [outdir]` | decrypt a directory |
| `keyfile [scandir]` | write a desktop-plugin keyfile |
| `scan` / `scantruncate [dir] [menu]` | rewrite kfxdedrm's own menu |

`Engine.locateIn` calls `test` and `Engine.decryptCommand` calls `dedrm`.
`dedrm_all` carries no per-book progress.

**The engine does not read the out-folder argument.** It parses
`dedrm <book>` and writes to `Config.OUT_DIR` whatever it is handed, which is
why that is a constant and not a setting. `Engine.decryptCommand` passes the
argument regardless, so a build that does read it writes to the same place and
needs no change here.

A port of `native/src/engine.rs`.
]]

local lfs = require("libs/libkoreader-lfs")
local util = require("util")

local Engine = {}

--- The engine extension's root, distinct from this plugin's.
Engine.EXTENSION_DIR = "/mnt/us/extensions/kfxdedrm"
--- Where the engine's four ABI builds live.
Engine.BIN_DIR = Engine.EXTENSION_DIR .. "/bin"

--- Shown verbatim: the menu has no browser and the string is transcribed by
--- hand.
Engine.RELEASES_URL = "github.com/Satsuoni/DeDRM_tools/releases"
--- The MOBI-capable asset. `kfxdedrm_kual.zip` covers KFX alone.
Engine.RELEASE_ASSET = "kfxdedrmmobi.zip"

--- The engine's four builds in probe order: hard-float first, soft-float
--- second, `_c11` ahead of `_old` within each.
Engine.ABI_VARIANTS = {
    "kfxdedrmhf_c11",
    "kfxdedrmhf_old",
    "kfxdedrm_old",
    "kfxdedrm_c11",
}

--- Why `Engine.locate` found nothing.
Engine.NOT_INSTALLED = "not_installed"
Engine.NO_WORKING_BUILD = "no_working_build"

--- The engine's two code paths, which `Engine.outputPath` names differently.
--- `kfx` is a KFX container keyed by a `.sdr` voucher sidecar; `mobi` is
--- `Engine.MOBI_EXTENSIONS`.
Engine.KFX = "kfx"
Engine.MOBI = "mobi"

--- The extensions the engine names as MOBI book candidates.
Engine.MOBI_EXTENSIONS = { "azw3", "azw4", "mobi" }

local function basename(path)
    return path:match("([^/]*)$")
end

--- `path`'s file name split at its last dot, the way `Path::file_stem` and
--- `Path::extension` split it: a name with no dot, or one whose only dot
--- leads it, is all stem.
function Engine.stemAndExtension(path)
    local name = basename(path)
    local stem, ext = name:match("^(.+)%.([^.]*)$")
    if not stem then return name, nil end
    return stem, ext
end

--- `dir` and `name` joined, with however many slashes `dir` ends in reduced
--- to the one between them.
function Engine.join(dir, name)
    return (dir:gsub("/*$", "")) .. "/" .. name
end

--- The format of the book at `path`, by extension, case-insensitively: a FAT
--- partition carries `.AZW3`.
function Engine.formatOf(path)
    local _, ext = Engine.stemAndExtension(path)
    if not ext then return nil end
    ext = ext:lower()
    if ext == "kfx" then
        return Engine.KFX
    end
    for _, mobi_ext in ipairs(Engine.MOBI_EXTENSIONS) do
        if ext == mobi_ext then return Engine.MOBI end
    end
    return nil
end

--- The engine's output for `book` under `out_dir`.
---
--- `Engine.KFX` takes the `.kfx-zip` extension. `Engine.MOBI` keeps its own
--- filename, copied into `out_dir` and patched in place.
function Engine.outputPath(book, out_dir)
    local format = Engine.formatOf(book)
    if not format then return nil end
    if format == Engine.KFX then
        local stem = Engine.stemAndExtension(book)
        return Engine.join(out_dir, stem .. ".kfx-zip")
    end
    return Engine.join(out_dir, basename(book))
end

--- `Engine.ABI_VARIANTS` under `dir`, in probe order.
function Engine.variantPaths(dir)
    local paths = {}
    for _, name in ipairs(Engine.ABI_VARIANTS) do
        table.insert(paths, Engine.join(dir, name))
    end
    return paths
end

--- The first `Engine.variantPaths` entry whose `test` exits 0.
---
--- Each variant targets a different ABI; three of the four fail to start on
--- any one device.
local function probe_in(dir)
    for _, exe in ipairs(Engine.variantPaths(dir)) do
        if lfs.attributes(exe, "mode") == "file" then
            local cmd = util.shell_escape({ exe, "test" }) .. " >/dev/null 2>&1"
            if os.execute(cmd) == 0 then
                return exe
            end
        end
    end
    return nil
end

--- The engine under `dir`, or `nil` and why there is none.
function Engine.locateIn(dir)
    if lfs.attributes(dir, "mode") ~= "directory" then
        return nil, Engine.NOT_INSTALLED
    end
    local exe = probe_in(dir)
    if not exe then
        return nil, Engine.NO_WORKING_BUILD
    end
    return exe
end

--- `Engine.locateIn` over `Engine.BIN_DIR`.
function Engine.locate()
    return Engine.locateIn(Engine.BIN_DIR)
end

--- `<exe> dedrm <book> <out_dir>`, with the engine's own output folded into
--- stdout so the caller can log what it said.
---
--- `out_dir` rides every call and the engine ignores it -- see this module's
--- header. Callers pass `Config.OUT_DIR`, which is where it writes anyway, and
--- it creates that folder itself.
function Engine.decryptCommand(exe, book, out_dir)
    return util.shell_escape({ exe, "dedrm", book, out_dir }) .. " 2>&1"
end

return Engine
