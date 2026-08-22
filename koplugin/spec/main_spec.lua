-- main.lua loaded against stubbed KOReader widgets: the menu it builds, and
-- every string it can be asked for without a device.
local harness = require("harness")
local check, eq = harness.check, harness.eq
local stubs = require("stubs")


local KfxDeDRM = assert(loadfile(harness.PLUGIN .. "/main.lua"))()
check("main.lua returns a plugin on a Kindle", KfxDeDRM.name == "kfxdedrm", KfxDeDRM.name)

-- `PluginLoader` copies every `_meta.lua` key but `name` onto the module.
-- `loadfile` above copies none, and the loop below does it.
local meta = assert(loadfile(harness.PLUGIN .. "/_meta.lua"))()
for key, value in pairs(meta) do
    if key ~= "name" then KfxDeDRM[key] = value end
end
check("the metadata carries a version", type(meta.version) == "string" and #meta.version > 0)
check("and a description for the plugin list",
    type(meta.description) == "string" and #meta.description > 0)

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
local labels = {}
local function walk(list, path)
    for i, item in ipairs(list) do
        rows = rows + 1
        local where = path .. "[" .. i .. "]"
        local label = item.text or (item.text_func and item.text_func())
        check("a label at " .. where, type(label) == "string" and #label > 0, label)
        if label then
            labels[label] = true
            -- A row that carries its setting is quoted by the part in front
            -- of its colon.
            local head = label:match("^(.-): ")
            if head then labels[head] = true end
        end
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

-- Nine rows at the top level: one page on a Scribe, none paged off alone.
check("the top level is one page", #root.sub_item_table <= 9, #root.sub_item_table)

-- Toggles move the config. Storing fails here (no /mnt/us) and must not throw.
local before = plugin.cfg.show_done
for _, item in ipairs(root.sub_item_table) do
    if item.text == "Show books already decrypted" then item.callback() end
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

-- One row fetches both add-ons, with `keep_menu_open` holding the menu.
local dep_rows = 0
for _, item in ipairs(root.sub_item_table) do
    if item.text == "Install or update the decryption tools" then
        dep_rows = dep_rows + 1
        check("the add-on row keeps the menu open", item.keep_menu_open == true)
    end
end
eq("one row covers both add-ons", dep_rows, 1)

-- One row reports what is installed, with `keep_menu_open` holding the menu.
local status_rows = 0
for _, item in ipairs(root.sub_item_table) do
    if item.text == "What's installed and where" then
        status_rows = status_rows + 1
        check("the status row keeps the menu open", item.keep_menu_open == true)
    end
end
eq("one row reports what is installed", status_rows, 1)

-- One row updates the plugin, with `keep_menu_open` holding the menu.
local self_rows = 0
for _, item in ipairs(root.sub_item_table) do
    if item.text == "Update this plugin" then
        self_rows = self_rows + 1
        check("the update row keeps the menu open", item.keep_menu_open == true)
    end
end
eq("and one row updates this plugin", self_rows, 1)

-- `pluginDir` and `installedVersion` for a copy `loadfile` brought in.
check("this copy names its own folder", plugin:pluginDir() == harness.PLUGIN, plugin:pluginDir())
check("and its own version", plugin:installedVersion() ~= nil, plugin:installedVersion())

-- fetchOne's outcomes, with the network stood in for.
local Install = require("lib.install")
local engine_source = Install.source("engine")
local real_available, real_run = Install.available, Install.run
local function nostep() end

-- `Install.RECORD_PATH` is a real file shared with native/. Here it points
-- under `harness.SPEC`, clear of /mnt/us.
Install.RECORD_PATH = harness.SPEC .. "/cache/installs.txt"
os.remove(Install.RECORD_PATH)

plugin.engine_probed, plugin.engine_exe = true, "/fake/bin/kfxdedrmhf_c11"

Install.available = function() return nil, "no reply" end
check("a release list that cannot be read is reported",
    plugin:fetchOne(engine_source, nostep):find("no reply", 1, true) ~= nil)

Install.available = function() return { version = "v10.0.30", name = "kfxdedrmmobi.zip" } end
Install.rememberTag("engine", "v10.0.30")
eq("an install already at that release is left alone",
    plugin:fetchOne(engine_source, nostep), "kfxdedrm: already at v10.0.30")

Install.rememberTag("engine", "v10.0.29")
Install.run = function() return nil, "download failed" end
check("a failed install says why",
    plugin:fetchOne(engine_source, nostep):find("download failed", 1, true) ~= nil)
eq("and the older tag stands", Install.installedTag("engine"), "v10.0.29")

Install.run = function(_source, release) return release.version end
eq("a fresh install names what landed",
    plugin:fetchOne(engine_source, nostep), "kfxdedrm: installed v10.0.30")
eq("and records it", Install.installedTag("engine"), "v10.0.30")

-- `aboutText` is the one place an installed release is named.
check("the status screen names the installed release",
    plugin:aboutText():find("v10.0.30", 1, true) ~= nil)

Install.available, Install.run = real_available, real_run
os.remove(Install.RECORD_PATH)
plugin.engine_probed, plugin.engine_exe = nil, nil

-- The strings a missing install shows.
local Engine = require("lib.engine")
local Convert = require("lib.convert")
local not_installed = plugin:engineMissingText(Engine.NOT_INSTALLED)
check("the not-installed screen names the asset", not_installed:find(Engine.RELEASE_ASSET, 1, true))
check("and the releases page", not_installed:find(Engine.RELEASES_URL, 1, true))
check("and where it goes", not_installed:find(Engine.EXTENSION_DIR, 1, true))
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

-- Every name a dialog quotes in curly quotes is a label the menu draws.
local dialogs = { not_installed, broken, bokai }
plugin.cfg.scan_dirs = {}
dialogs[#dialogs + 1] = plugin:emptyText()
plugin.cfg.scan_dirs = { "/mnt/us/documents" }
plugin.cfg.show_done = false
dialogs[#dialogs + 1] = plugin:emptyText()

local pointed = 0
for _, text in ipairs(dialogs) do
    for quoted in text:gmatch("“(.-)”") do
        pointed = pointed + 1
        check("a dialog points at the row " .. quoted, labels[quoted] == true, quoted)
    end
end
check("some dialog does point at a row", pointed >= 5, pointed)

-- `foldersLabel`, `formatsLabel` and `alsoSaveLabel`: the setting, as one line.
plugin.cfg.scan_dirs = {}
eq("no folder picked", plugin:foldersLabel(), "none")
plugin.cfg.scan_dirs = { "/mnt/us/documents/Downloads/Items01" }
eq("one folder is named, off the documents folder", plugin:foldersLabel(), "Downloads/Items01")
plugin.cfg.scan_dirs = { "/a", "/b" }
eq("more than one is counted", plugin:foldersLabel(), "2 folders")

plugin.cfg.types_kfx, plugin.cfg.types_mobi = true, true
eq("both formats listed", plugin:formatsLabel(), "KFX, MOBI")
plugin.cfg.types_kfx, plugin.cfg.types_mobi = true, false
eq("one of them", plugin:formatsLabel(), "KFX")
plugin.cfg.types_kfx, plugin.cfg.types_mobi = false, false
eq("neither", plugin:formatsLabel(), "none")

plugin.cfg.pack_kfx, plugin.cfg.convert_epub = true, true
eq("both extra formats", plugin:alsoSaveLabel(), "KFX, EPUB")
plugin.cfg.pack_kfx, plugin.cfg.convert_epub = false, true
eq("one of them", plugin:alsoSaveLabel(), "EPUB")
plugin.cfg.pack_kfx, plugin.cfg.convert_epub = false, false
eq("neither", plugin:alsoSaveLabel(), "none")

-- `text_func` draws that line on the three rows that carry a setting.
plugin.cfg.scan_dirs = { "/mnt/us/documents" }
plugin.cfg.types_kfx = true
plugin.cfg.convert_epub = true
local valued = {}
for _, item in ipairs(root.sub_item_table) do
    if item.text_func then valued[#valued + 1] = item.text_func() end
end
eq("three rows carry their setting", #valued, 3)
eq("the folder row", valued[1], "Folders to scan: documents")
eq("the format row", valued[2], "Formats to look for: KFX")
eq("the extra-format row", valued[3], "Also save as: EPUB")

local about = plugin:aboutText()
check("the about screen opens on this plugin's own build",
    about:find(meta.version, 1, true) ~= nil, about)
check("the about screen names the output folder", about:find("/mnt/us/dedrm", 1, true))
check("and the settings file", about:find("config.txt", 1, true))

-- Off a Kindle the plugin declines to load at all.
package.loaded["lib.config"] = nil
stubs["device"].kindle = false
local elsewhere = assert(loadfile(harness.PLUGIN .. "/main.lua"))()
check("nothing loads off a Kindle", elsewhere.disabled == true)
stubs["device"].kindle = true

return harness.report()
