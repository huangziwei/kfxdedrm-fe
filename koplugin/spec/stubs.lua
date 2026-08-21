-- The KOReader modules `kfxdedrm.koplugin` requires, enough of each to run its
-- own code on a host. `lfs` goes through `stat`, the archive reader through
-- `unzip`; neither is what the device uses, and neither is shipped.

local function q(s)
    return "'" .. s:gsub("'", "'\\''") .. "'"
end

--------------------------------------------------------------------------------
-- libs/libkoreader-lfs
--------------------------------------------------------------------------------

local lfs = {}

-- `stat` is BSD on a Mac and GNU on Linux, and the two spell their format flag
-- differently. Probed once, on a path that is always there: GNU answers `-c`
-- and BSD prints its usage to stderr instead.
local stat_flags
local function stat_format()
    if not stat_flags then
        local probe = io.popen("stat -c '%F' . 2>/dev/null")
        local kind = probe and probe:read("*line")
        if probe then probe:close() end
        stat_flags = (kind and kind ~= "") and "-L -c '%F|%s|%Y'" or "-L -f '%HT|%z|%m'"
    end
    return stat_flags
end

-- The two name the same kinds in different words, and GNU calls a zero-byte
-- file a "regular empty file".
local function mode_of(kind)
    kind = kind:lower()
    if kind:find("directory", 1, true) then return "directory" end
    if kind:find("regular", 1, true) then return "file" end
    return "other"
end

function lfs.attributes(path, what)
    local pipe = io.popen("stat " .. stat_format() .. " " .. q(path) .. " 2>/dev/null")
    local line = pipe and pipe:read("*line")
    if pipe then pipe:close() end
    if not line or line == "" then return nil end
    local kind, size, mtime = line:match("^(.-)|(%d+)|(%d+)$")
    if not kind then return nil end
    local attrs = {
        mode = mode_of(kind),
        size = tonumber(size),
        modification = tonumber(mtime),
    }
    if what then return attrs[what] end
    return attrs
end

function lfs.dir(path)
    local pipe = io.popen("ls -a " .. q(path) .. " 2>/dev/null")
    if not pipe then error("cannot list " .. path) end
    local names = {}
    for name in pipe:lines() do
        names[#names + 1] = name
    end
    pipe:close()
    local i = 0
    return function()
        i = i + 1
        return names[i]
    end
end

function lfs.mkdir(path)
    return os.execute("mkdir " .. q(path) .. " 2>/dev/null") == 0
end

--------------------------------------------------------------------------------
-- util
--------------------------------------------------------------------------------

local util = {}

function util.shell_escape(args)
    local escaped = {}
    for _, arg in ipairs(args) do
        escaped[#escaped + 1] = "'" .. arg:gsub("'", "'\\''") .. "'"
    end
    return table.concat(escaped, " ")
end

function util.getFriendlySize(size)
    return string.format("%.1f KB", (tonumber(size) or 0) / 1024)
end

--------------------------------------------------------------------------------
-- the rest
--------------------------------------------------------------------------------

local logger = {}
logger.quiet = true
local function log(level)
    return function(...)
        if logger.quiet then return end
        local parts = { "[" .. level .. "]" }
        for i = 1, select("#", ...) do
            parts[#parts + 1] = tostring((select(i, ...)))
        end
        print(table.concat(parts, " "))
    end
end
logger.info, logger.warn, logger.dbg, logger.err = log("i"), log("w"), log("d"), log("e")

local gettext = setmetatable({}, { __call = function(_, s) return s end })
gettext.pgettext = function(_ctx, s) return s end

local ffiUtil = {}
function ffiUtil.template(str, ...)
    local args = { ... }
    return (str:gsub("%%(%d+)", function(n)
        return tostring(args[tonumber(n)])
    end))
end

local modules = {
    ["libs/libkoreader-lfs"] = lfs,
    ["util"] = util,
    ["logger"] = logger,
    ["gettext"] = gettext,
    ["ffi/util"] = ffiUtil,
}

table.insert(package.loaders, 1, function(name)
    if modules[name] then
        return function() return modules[name] end
    end
    return nil
end)

--------------------------------------------------------------------------------
-- enough of the widget world for main.lua to load and answer questions
--------------------------------------------------------------------------------

local Device = { kindle = true }
function Device:isKindle() return self.kindle end

local WidgetContainer = {}
function WidgetContainer:extend(subclass)
    subclass = subclass or {}
    setmetatable(subclass, { __index = self })
    return subclass
end
function WidgetContainer:new(o)
    o = o or {}
    setmetatable(o, { __index = self })
    if o.init then o:init() end
    return o
end

local shown = {}
local UIManager = {
    shown = shown,
    show = function(_self, w) shown[#shown + 1] = w end,
    close = function() end,
    forceRePaint = function() end,
}

local function widget_class()
    local W = {}
    function W:new(o)
        o = o or {}
        setmetatable(o, { __index = W })
        return o
    end
    return W
end

local aux_providers = {}
local DocumentRegistry = {
    aux = aux_providers,
    addAuxProvider = function(_self, p) aux_providers[p.provider] = p end,
}

local Trapper = {
    wrap = function(_self, f) return f() end,
    info = function() return true end,
    clear = function() end,
    setPausedText = function() end,
}

modules["device"] = Device
modules["ui/widget/container/widgetcontainer"] = WidgetContainer
modules["ui/uimanager"] = UIManager
modules["ui/widget/infomessage"] = widget_class()
modules["ui/widget/menu"] = widget_class()
modules["document/documentregistry"] = DocumentRegistry
modules["ui/trapper"] = Trapper

--------------------------------------------------------------------------------
-- ffi/archiver over the `unzip` binary, enough to run Install.unpack on a host
--------------------------------------------------------------------------------

local Reader = {}
Reader.__index = Reader

function Reader:new()
    return setmetatable({}, Reader)
end

function Reader:open(path)
    local probe = io.popen("unzip -Z1 " .. q(path) .. " 2>/dev/null")
    if not probe then return nil end
    self.entries = {}
    for name in probe:lines() do
        self.entries[#self.entries + 1] = {
            path = name,
            mode = name:sub(-1) == "/" and "directory" or "file",
        }
    end
    probe:close()
    if #self.entries == 0 then
        self.err = "not an archive"
        return nil
    end
    self.path = path
    return true
end

function Reader:iterate()
    local i = 0
    return function()
        i = i + 1
        return self.entries[i]
    end
end

function Reader:extractToPath(key, dest)
    local dir = dest:match("^(.*)/[^/]*$")
    if dir then os.execute("mkdir -p " .. q(dir)) end
    return os.execute("unzip -p " .. q(self.path) .. " " .. q(key) .. " > " .. q(dest) .. " 2>/dev/null") == 0
end

function Reader:close() end

modules["ffi/archiver"] = { Reader = Reader }

return modules
