--[[--
Which folders `lib/scan` reads and what it lists out of them.

The `key = value` file `native/src/config.rs` writes, at the path it writes it
to. One device carries both frontends and one set of settings between them,
which is why this is not a KOReader `LuaSettings`: a folder selected in the
KUAL app is selected here, and back.

`Config.parse` has no failure mode; `Config.sanitized` is the one filter it
applies. `Config.render` has to match `Config::render` byte for byte, or each
frontend rewrites the other's file on every save.

Where the engine writes is not a setting -- see `Config.OUT_DIR`.
]]

local lfs = require("libs/libkoreader-lfs")
local logger = require("logger")

local Config = {}

--- Purchased downloads on current firmware, and the folder a fresh install
--- reads.
Config.ITEMS01_DIR = "/mnt/us/documents/Downloads/Items01"
--- Library root: purchases on older models, mixed with sideloads.
Config.DOCUMENTS_DIR = "/mnt/us/documents"
--- Where the engine writes, and the one place decrypted books land.
---
--- Not a setting: the engine hardcodes this and ignores the out-folder
--- argument it is handed.
Config.OUT_DIR = "/mnt/us/dedrm"
--- The bokai add-on's root, named in the rendered comments.
Config.BOKAI_DIR = "/mnt/us/extensions/bokai"
--- The settings file, shared with the native frontend.
Config.PATH = "/mnt/us/extensions/kfxdedrm-fe/config.txt"

--- Everything the settings menu controls.
function Config.default()
    return {
        scan_dirs = { Config.ITEMS01_DIR },
        types_kfx = true,
        types_mobi = true,
        -- Off: the add-on the two of them run is not part of this install.
        pack_kfx = false,
        convert_epub = false,
        show_done = true,
    }
end

--- `true`/`false` and the spellings a hand-edited file carries. `nil` leaves
--- the caller's default.
local function parse_bool(v)
    v = v:lower()
    if v == "true" or v == "yes" or v == "on" or v == "1" then
        return true
    elseif v == "false" or v == "no" or v == "off" or v == "0" then
        return false
    end
    return nil
end

local function contains(list, value)
    for _, v in ipairs(list) do
        if v == value then return true end
    end
    return false
end

--- Drops any `scan_dirs` entry that would misbehave, and any repeat of one
--- that would not.
---
--- A relative path resolves against the engine's working directory.
--- `OUT_DIR` holds the engine's own output, and `Engine.outputPath` of a MOBI
--- there is the file itself -- the engine would copy it onto itself.
function Config.sanitized(cfg)
    local kept = {}
    for _, dir in ipairs(cfg.scan_dirs) do
        if dir:sub(1, 1) == "/" and dir ~= Config.OUT_DIR and not contains(kept, dir) then
            table.insert(kept, dir)
        end
    end
    cfg.scan_dirs = kept
    return cfg
end

--- `key = value` lines; blank lines, `#` comments and lines without `=` are
--- skipped. An unreadable value leaves that one field at its default.
---
--- `scan_dir` carries one folder and may repeat. A file naming none at all
--- takes `Config.default`'s; one naming it with an empty value has deselected
--- every folder, which `Config.render` writes back the same way.
function Config.parse(text)
    local cfg = Config.default()
    local scan_dirs = {}
    local named_a_folder = false

    for line in (text .. "\n"):gmatch("(.-)\r?\n") do
        line = line:match("^%s*(.-)%s*$")
        if line ~= "" and line:sub(1, 1) ~= "#" then
            local key, value = line:match("^([^=]*)=(.*)$")
            if key then
                key = key:match("^%s*(.-)%s*$")
                value = value:match("^%s*(.-)%s*$")
                if key == "scan_dir" then
                    named_a_folder = true
                    if value ~= "" then
                        table.insert(scan_dirs, value)
                    end
                elseif key == "types_kfx" then
                    local b = parse_bool(value)
                    if b ~= nil then cfg.types_kfx = b end
                elseif key == "types_mobi" then
                    local b = parse_bool(value)
                    if b ~= nil then cfg.types_mobi = b end
                elseif key == "pack_kfx" then
                    local b = parse_bool(value)
                    if b ~= nil then cfg.pack_kfx = b end
                elseif key == "convert_epub" then
                    local b = parse_bool(value)
                    if b ~= nil then cfg.convert_epub = b end
                elseif key == "show_done" then
                    local b = parse_bool(value)
                    if b ~= nil then cfg.show_done = b end
                end
            end
        end
    end
    if named_a_folder then
        cfg.scan_dirs = scan_dirs
    end
    return Config.sanitized(cfg)
end

--- The file format, comments included.
---
--- The wording is the native frontend's, down to the panel it names and the
--- grid it describes: this writes the same file, and a reworded copy would
--- churn every line of it each time the other frontend saved.
function Config.render(cfg)
    local folders = {}
    if #cfg.scan_dirs == 0 then
        table.insert(folders, "scan_dir =\n")
    end
    for _, dir in ipairs(cfg.scan_dirs) do
        table.insert(folders, "scan_dir = " .. dir .. "\n")
    end

    return table.concat({
        "# kfxdedrm-fe settings. Rewritten whenever the Settings panel is used, so\n",
        "# comments you add here will not survive; the values will.\n",
        "\n",
        "# Where to look for books, one line per folder, each read one level deep.\n",
        "# Settings offers a chip for every folder under ", Config.DOCUMENTS_DIR, " that holds\n",
        "# a DRM'd book. An empty value selects none.\n",
        table.concat(folders),
        "\n",
        "# Which formats to list. KFX books are listed when their .sdr voucher is\n",
        "# present; MOBI-family books when their own header says they carry DRM, so\n",
        "# DRM-free sideloads never appear.\n",
        "types_kfx = ", tostring(cfg.types_kfx), "\n",
        "types_mobi = ", tostring(cfg.types_mobi), "\n",
        "\n",
        "# Extra formats, written into ", Config.OUT_DIR, " beside the engine's own output. Both\n",
        "# need the bokai add-on at ", Config.BOKAI_DIR, "; without it they are ignored.\n",
        "#   pack_kfx      merge the .kfx-zip bundle into one .kfx container\n",
        "#   convert_epub  convert the book to .epub\n",
        "pack_kfx = ", tostring(cfg.pack_kfx), "\n",
        "convert_epub = ", tostring(cfg.convert_epub), "\n",
        "\n",
        "# Keep finished books in the grid, marked with a check.\n",
        "show_done = ", tostring(cfg.show_done), "\n",
    })
end

--- `Config.parse` of `path`, or `Config.default`.
function Config.load(path)
    path = path or Config.PATH
    local f = io.open(path, "r")
    if not f then
        return Config.default()
    end
    local text = f:read("*all")
    f:close()
    return Config.parse(text or "")
end

--- `Config.render` to `path`, creating its parent.
function Config.store(cfg, path)
    path = path or Config.PATH
    local dir = path:match("^(.*)/[^/]*$")
    if dir and lfs.attributes(dir, "mode") ~= "directory" then
        -- One level: the parent of the settings file, not a whole tree.
        lfs.mkdir(dir)
    end
    local f = io.open(path, "w")
    if not f then
        logger.warn("kfxdedrm: cannot write", path)
        return false
    end
    f:write(Config.render(cfg))
    f:close()
    return true
end

--- False once no folder is selected or every format is off.
function Config.listsAnything(cfg)
    return #cfg.scan_dirs > 0 and (cfg.types_kfx or cfg.types_mobi)
end

return Config
