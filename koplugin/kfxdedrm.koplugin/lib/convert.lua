--[[--
`Convert.locate` the bokai converter under `Convert.BIN_DIR`, `Convert.targets`
for what the settings ask of it, `Convert.convertCommand` to run one step.

bokai is an add-on, not a dependency: `Convert.locate` returning `nil` leaves
the plugin decrypting and doing nothing else. It is installed by hand, the way
`lib/engine`'s extension is, and lands beside it under `/mnt/us/extensions/`.

The converter's command surface:

| invocation | effect |
|:--|:--|
| `--version` | exits 0 if this build runs on this device |
| `convert <in> <out>` | both formats read off the two extensions |

Every conversion here starts from `Engine.outputPath` -- `<stem>.kfx-zip` for a
KFX book, the book's own name for a MOBI-family one -- and writes beside it,
inside `Config.OUT_DIR`.

A port of `native/src/convert.rs`.
]]

local lfs = require("libs/libkoreader-lfs")
local util = require("util")
local _ = require("gettext")

local Engine = require("lib.engine")

local Convert = {}

--- The add-on extension's root, distinct from this plugin's and the engine's.
Convert.EXTENSION_DIR = "/mnt/us/extensions/bokai"
--- Where the zip installs bokai's ABI builds.
Convert.BIN_DIR = Convert.EXTENSION_DIR .. "/bin"

--- Shown verbatim by the settings menu: it has no browser and the string is
--- transcribed by hand.
Convert.RELEASES_URL = "github.com/huangziwei/sidle/releases"
--- The asset, `*` standing for the version. bokai versions on its own line and
--- moves without this plugin moving, so no one version belongs here.
Convert.RELEASE_ASSET = "bokai-*-kindle.zip"

--- bokai's two builds in `Convert.locateIn` order: hard-float first, soft-float
--- second. One zip carries both and a device starts one of them.
Convert.ABI_VARIANTS = { "bokai", "bokai-armsf" }

--- Extensions bokai reads.
---
--- `azw4` is an `Engine.MOBI_EXTENSIONS` entry bokai's own format detection
--- does not name; a step over one would only fail.
local READABLE = { "kfx-zip", "kfx", "azw3", "mobi" }

--- The extra format a step produces.
Convert.KFX = "kfx"
Convert.EPUB = "epub"

--- The extension a step's output takes.
function Convert.extension(kind)
    return kind == Convert.KFX and "kfx" or "epub"
end

--- Banner line while the step runs.
function Convert.progress(kind)
    if kind == Convert.KFX then
        return _("Packing as KFX…")
    end
    return _("Converting to EPUB…")
end

--- Name in a result banner.
function Convert.label(kind)
    return kind == Convert.KFX and "KFX" or "EPUB"
end

--- `exe`, if it is a file whose `--version` exits 0.
---
--- The run costs one process and rules out a build for the wrong ABI, which
--- otherwise fails once per book with the banner mid-decrypt.
function Convert.locateAt(exe)
    if lfs.attributes(exe, "mode") ~= "file" then return nil end
    local cmd = util.shell_escape({ exe, "--version" }) .. " >/dev/null 2>&1"
    if os.execute(cmd) ~= 0 then return nil end
    return exe
end

--- `Convert.ABI_VARIANTS` under `dir`, in probe order.
function Convert.variantPaths(dir)
    local paths = {}
    for _, name in ipairs(Convert.ABI_VARIANTS) do
        table.insert(paths, Engine.join(dir, name))
    end
    return paths
end

--- The first `Convert.variantPaths` entry under `dir` that `Convert.locateAt`
--- accepts.
---
--- Each variant targets a different float ABI, so at most one of them starts
--- on any one device.
function Convert.locateIn(dir)
    for _, exe in ipairs(Convert.variantPaths(dir)) do
        local found = Convert.locateAt(exe)
        if found then return found end
    end
    return nil
end

--- `Convert.locateIn` over `Convert.BIN_DIR`.
function Convert.locate()
    return Convert.locateIn(Convert.BIN_DIR)
end

--- `path`'s extension equals `ext`, case-insensitively: a FAT partition
--- carries `.AZW3`.
local function extension_is(path, ext)
    local _stem, found = Engine.stemAndExtension(path)
    return found ~= nil and found:lower() == ext
end

--- The engine's KFX output, whose extension no other format shares.
local function is_kfx_zip(path)
    return extension_is(path, "kfx-zip")
end

--- Whether bokai has an importer for `path`'s extension.
local function readable(path)
    for _, ext in ipairs(READABLE) do
        if extension_is(path, ext) then return true end
    end
    return false
end

--- `path` with its extension replaced by `ext`.
local function with_extension(path, ext)
    local dir = path:match("^(.*)/[^/]*$")
    local stem = Engine.stemAndExtension(path)
    local name = stem .. "." .. ext
    return dir and Engine.join(dir, name) or name
end

--- The two switches, resolved against what is installed.
---
--- A switch left on in the file names a binary that is not there, and
--- `Scan.book.done` would then never come true for any book.
function Convert.targets(cfg, converter)
    if not converter then
        return { kfx = false, epub = false }
    end
    return { kfx = cfg.pack_kfx, epub = cfg.convert_epub }
end

function Convert.any(targets)
    return targets.kfx or targets.epub
end

--- The conversions for `decrypted`, in run order.
---
--- `decrypted` is `Engine.outputPath`'s result. A `Convert.KFX` step applies to
--- a `.kfx-zip` alone: a MOBI-family book is copied under its own name and has
--- no bundle to merge.
function Convert.steps(targets, decrypted)
    local steps = {}
    if not Convert.any(targets) or not readable(decrypted) then
        return steps
    end

    if targets.kfx and is_kfx_zip(decrypted) then
        table.insert(steps, {
            kind = Convert.KFX,
            input = decrypted,
            output = with_extension(decrypted, Convert.extension(Convert.KFX)),
        })
    end
    if targets.epub then
        -- From the packed KFX when this run produces one: the merge is the
        -- cheaper half of the two, and the EPUB then comes off a single
        -- container rather than the bundle a second time.
        local input = steps[1] and steps[1].output or decrypted
        table.insert(steps, {
            kind = Convert.EPUB,
            input = input,
            output = with_extension(decrypted, Convert.extension(Convert.EPUB)),
        })
    end
    return steps
end

--- What `Convert.steps` writes. `Scan.book.done` waits on all of it.
function Convert.outputs(targets, decrypted)
    local outputs = {}
    for _, step in ipairs(Convert.steps(targets, decrypted)) do
        table.insert(outputs, step.output)
    end
    return outputs
end

--- `<exe> convert <input> <output>`.
---
--- bokai takes both formats off the extensions, so neither `-f` nor `-t` rides
--- the call. Its progress goes to stderr, folded into stdout here so the
--- caller can log what it said.
function Convert.convertCommand(exe, step)
    return util.shell_escape({ exe, "convert", step.input, step.output }) .. " 2>&1"
end

return Convert
