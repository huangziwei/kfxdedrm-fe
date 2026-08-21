-- main.lua loaded against stubbed KOReader widgets: the menu it builds, and
-- every string it can be asked for without a device.
local harness = require("harness")
local check, eq = harness.check, harness.eq
local stubs = require("stubs")


local KfxDeDRM = assert(loadfile(harness.PLUGIN .. "/main.lua"))()
check("main.lua returns a plugin on a Kindle", KfxDeDRM.name == "kfxdedrm", KfxDeDRM.name)

local registered
local plugin = KfxDeDRM:new{
    ui = { menu = { registerToMainMenu = function(_self, p) registered = p end } },
}
check("it registers itself with the main menu", registered == plugin)
check("it registers an Open with… provider", stubs["document/documentregistry"].aux.kfxdedrm ~= nil)
local aux = stubs["document/documentregistry"].aux.kfxdedrm
check("the provider is auxiliary (has an order)", aux.order ~= nil)
check("no default-handler offer", aux.disable_file == true and aux.disable_type == true)

-- The extensions the Open with… dialog offers this for.
check("offers itself for a kfx", plugin:isFileTypeSupported("/d/Book.kfx"))
check("offers itself for an azw3", plugin:isFileTypeSupported("/d/Book.azw3"))
check("not for an epub", not plugin:isFileTypeSupported("/d/Book.epub"))
check("not for the engine's own output", not plugin:isFileTypeSupported("/d/Book.kfx-zip"))

-- Menu shape: every row reachable, no row without a label.
local items = {}
plugin:addToMainMenu(items)
local root = items.kfxdedrm
check("the menu lands under Tools", root.sorting_hint == "more_tools", root.sorting_hint)
check("with a name", type(root.text) == "string" and #root.text > 0)

local rows = 0
local function walk(list, path)
    for i, item in ipairs(list) do
        rows = rows + 1
        local where = path .. "[" .. i .. "]"
        local label = item.text or (item.text_func and item.text_func())
        check("a label at " .. where, type(label) == "string" and #label > 0, label)
        check("something to do at " .. where,
            item.callback ~= nil or item.sub_item_table ~= nil
            or item.sub_item_table_func ~= nil or item.enabled == false,
            label)
        if item.sub_item_table then walk(item.sub_item_table, where) end
        if item.sub_item_table_func then walk(item.sub_item_table_func(), where .. "()") end
        if item.checked_func then item.checked_func() end
    end
end
walk(root.sub_item_table, "menu")
check("the menu has rows", rows >= 8, rows)

-- Toggles move the config. Storing fails here (no /mnt/us) and must not throw.
local before = plugin.cfg.show_done
for _, item in ipairs(root.sub_item_table) do
    if item.text == "Keep finished books listed" then item.callback() end
end
check("a toggle flips its setting", plugin.cfg.show_done == (not before))

plugin.cfg.scan_dirs = {}
plugin:toggleScanned("/mnt/us/documents/Sidle")
check("a folder can be selected", plugin:isScanned("/mnt/us/documents/Sidle"))
plugin:toggleScanned("/mnt/us/documents/Sidle")
check("and deselected", not plugin:isScanned("/mnt/us/documents/Sidle"))

-- Summaries, straight off `batch_summary`.
eq("nothing to do", KfxDeDRM.summary(0, 0, 0), "Nothing to do")
eq("all decrypted", KfxDeDRM.summary(3, 0, 0), "Decrypted 3")
eq("all failed", KfxDeDRM.summary(0, 2, 0), "All 2 failed\nSee the log")
eq("some of each", KfxDeDRM.summary(2, 1, 0), "Decrypted 2, 1 failed\nSee the log")
eq("a stopped batch names what it left", KfxDeDRM.summary(2, 0, 3), "Decrypted 2\nStopped, 3 left")
eq("a stopped batch that also failed", KfxDeDRM.summary(1, 1, 2),
    "Decrypted 1, 1 failed\nStopped, 2 left — see the log")

-- One row fetches both add-ons, and it leaves the menu standing: the tap after
-- it should be "Books…", not another descent through the menu.
local dep_rows = 0
for _, item in ipairs(root.sub_item_table) do
    if item.text and item.text:find("kfxdedrm and bokai", 1, true) then
        dep_rows = dep_rows + 1
        check("the add-on row keeps the menu open", item.keep_menu_open == true)
    end
end
eq("one row covers both add-ons", dep_rows, 1)

-- fetchOne's outcomes, with the network stood in for.
local Install = require("lib.install")
local engine_source = Install.source("engine")
local real_available, real_run = Install.available, Install.run
local function nostep() end

plugin.engine_probed, plugin.engine_exe = true, "/fake/bin/kfxdedrmhf_c11"

Install.available = function() return nil, "no reply" end
check("a release list that cannot be read is reported",
    plugin:fetchOne(engine_source, nostep):find("no reply", 1, true) ~= nil)

Install.available = function() return { tag = "v10.0.30", name = "kfxdedrmmobi.zip" } end
Install.rememberTag("engine", "v10.0.30")
eq("an install already at that release is left alone",
    plugin:fetchOne(engine_source, nostep), "kfxdedrm: already at v10.0.30")

Install.rememberTag("engine", "v10.0.29")
Install.run = function() return nil, "download failed" end
check("a failed install says why",
    plugin:fetchOne(engine_source, nostep):find("download failed", 1, true) ~= nil)
eq("and the older tag stands", Install.installedTag("engine"), "v10.0.29")

Install.run = function(_source, release) return release.tag end
eq("a fresh install names what landed",
    plugin:fetchOne(engine_source, nostep), "kfxdedrm: installed v10.0.30")
eq("and records it", Install.installedTag("engine"), "v10.0.30")

-- The status screen is the one place an installed release is named.
check("the status screen names the installed release",
    plugin:aboutText():find("v10.0.30", 1, true) ~= nil)

Install.available, Install.run = real_available, real_run
plugin.engine_probed, plugin.engine_exe = nil, nil

-- The strings a missing install shows.
local Engine = require("lib.engine")
local Convert = require("lib.convert")
local not_installed = plugin:engineMissingText(Engine.NOT_INSTALLED)
check("the not-installed screen names the asset", not_installed:find(Engine.RELEASE_ASSET, 1, true))
check("and the releases page", not_installed:find(Engine.RELEASES_URL, 1, true))
check("and where it goes", not_installed:find(Engine.EXTENSION_DIR, 1, true))
check("and points at the row that fetches it",
    not_installed:find("Download or update kfxdedrm and bokai", 1, true) ~= nil)
local broken = plugin:engineMissingText(Engine.NO_WORKING_BUILD)
check("a broken install says re-download", broken:find("Re%-download") ~= nil)
check("and counts the builds it tried", broken:find("4", 1, true) ~= nil)
local bokai = plugin:bokaiMissingText()
check("the bokai screen names its asset", bokai:find(Convert.RELEASE_ASSET, 1, true))
check("and its releases page", bokai:find(Convert.RELEASES_URL, 1, true))

-- Empty-list wording, one per reason.
plugin.cfg.scan_dirs = {}
check("nothing selected says so", plugin:emptyText():find("Nothing is being listed", 1, true))
plugin.cfg.scan_dirs = { "/mnt/us/documents" }
plugin.cfg.show_done = true
check("a folder with no books names the folder",
    plugin:emptyText():find("/mnt/us/documents", 1, true))
plugin.cfg.show_done = false
check("hidden finished books say so", plugin:emptyText():find("hidden", 1, true))

plugin.cfg.scan_dirs = { "/a", "/b", "/c" }
eq("many folders are counted, not named", plugin:foldersSummary(), "3 folders")
plugin.cfg.scan_dirs = {}
eq("no folder at all", plugin:foldersSummary(), "no folder")

local about = plugin:aboutText()
check("the about screen names the output folder", about:find("/mnt/us/dedrm", 1, true))
check("and the settings file", about:find("config.txt", 1, true))

-- Off a Kindle the plugin declines to load at all.
package.loaded["lib.config"] = nil
stubs["device"].kindle = false
local elsewhere = assert(loadfile(harness.PLUGIN .. "/main.lua"))()
check("nothing loads off a Kindle", elsewhere.disabled == true)
stubs["device"].kindle = true

return harness.report()
