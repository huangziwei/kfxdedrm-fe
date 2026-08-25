--[[--
Rewrites the settings fixtures `ports_spec` and `native/tests/
shared_settings_file.rs` both read.

    koplugin/spec/regen-fixtures.sh

Each fixture is written by `Config.render` and read back by both suites:
`ports_spec` compares Lua's output to it, `shared_settings_file.rs` compares
Rust's. Neither suite needs the other language.

Run this when `Config.render` changes. The diff it leaves is the whole of what
a settings file gains or loses; `native/tests/shared_settings_file.rs` then
judges whether `Config::render` agrees.
]]

local harness = require("harness")
local Config = require("lib.config")

--- One fixture, reported by name and by whether it moved.
local function write(name, text)
    local path = harness.SPEC .. "/fixtures/" .. name
    local before = io.open(path, "r")
    local was = before and before:read("*all") or nil
    if before then before:close() end

    local f = assert(io.open(path, "w"))
    f:write(text)
    f:close()
    print((was == text and "  unchanged  " or "  rewritten  ") .. name)
end

write("config-default.txt", Config.render(Config.default()))
write("config-no-folder.txt", Config.render({
    scan_dirs = {},
    out_dir = "/mnt/us/documents/dedrm",
    types_kfx = false,
    types_mobi = true,
    pack_kfx = true,
    convert_epub = true,
    show_done = false,
}))
