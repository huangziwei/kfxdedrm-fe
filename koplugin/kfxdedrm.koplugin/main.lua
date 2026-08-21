--[[--
A KOReader frontend for the kfxdedrm engine on a jailbroken Kindle.

The other half of this repository is `native/`, a standalone KUAL app with its
own framebuffer UI. This drives the same engine, reads and writes the same
settings file, and puts the result in the same folder. What it adds is where it
runs: with the bokai add-on installed, a book goes from DRM'd KFX to an EPUB
KOReader can open without leaving KOReader.

- `lib/config`, `lib/engine`, `lib/mobi`, `lib/scan` -- which books are listed,
  which engine build runs, what it receives, where its output lands.
- `lib/convert` -- the optional bokai add-on, and the extra formats the settings
  ask it for beside that output.

Nothing here decrypts anything: every book is one `dedrm` run of the engine
binary, and the engine is installed separately.
]]

local Device = require("device")

-- Every path here is under /mnt/us, and the engine ships as ARM builds against
-- the Kindle's own linker. There is nothing for this to drive anywhere else.
if not Device:isKindle() then
    return { disabled = true }
end

local DocumentRegistry = require("document/documentregistry")
local InfoMessage = require("ui/widget/infomessage")
local Menu = require("ui/widget/menu")
local Trapper = require("ui/trapper")
local UIManager = require("ui/uimanager")
local WidgetContainer = require("ui/widget/container/widgetcontainer")
local ffiUtil = require("ffi/util")
local lfs = require("libs/libkoreader-lfs")
local logger = require("logger")
local util = require("util")
local _ = require("gettext")
local T = ffiUtil.template

local Config = require("lib.config")
local Convert = require("lib.convert")
local Engine = require("lib.engine")
local Install = require("lib.install")
local Scan = require("lib.scan")

local KfxDeDRM = WidgetContainer:extend{
    name = "kfxdedrm",
    fullname = _("KFX DeDRM"),
}

--- `io.popen` under LuaJIT hands back no exit status, so the shell reports it
--- on a line of its own after the output.
local EXIT_MARKER = "__kfxdedrm_exit="

local function exists(path)
    return lfs.attributes(path, "mode") ~= nil
end

--- `cmd` run to its own exit: whether it succeeded, and what it wrote.
---
--- This blocks. Neither the engine nor bokai is interruptible once started --
--- `native/` polls the touchscreen across a run only to keep its own queue
--- drained, and stops between books, not inside one -- so a tap here queues up
--- and is read by the next `Trapper:info`.
local function run(cmd)
    logger.info("kfxdedrm: run", cmd)
    local pipe = io.popen(cmd .. "; echo " .. EXIT_MARKER .. "$?", "r")
    if not pipe then
        logger.warn("kfxdedrm: could not start", cmd)
        return false
    end
    local out = pipe:read("*all") or ""
    pipe:close()

    local status = out:match(EXIT_MARKER .. "(%d+)")
    for line in out:gmatch("[^\r\n]+") do
        if not line:find(EXIT_MARKER, 1, true) then
            logger.info("kfxdedrm:", line)
        end
    end
    logger.info("kfxdedrm: exit", status or "?")
    return status == "0"
end

--- `remove`, with an already-absent path reading as success.
local function remove_if_present(path)
    if not exists(path) then return true end
    return os.remove(path) and true or false
end

function KfxDeDRM:init()
    self.cfg = Config.load()
    self:registerDocumentRegistryAuxProvider()
    self.ui.menu:registerToMainMenu(self)
end

--------------------------------------------------------------------------------
-- What is installed
--------------------------------------------------------------------------------

--- The engine, or `nil` and why there is none.
---
--- Probed once and remembered: `Engine.locate` spawns one process per ABI
--- variant, and three of the four fail to start on any one device.
function KfxDeDRM:getEngine()
    if not self.engine_probed then
        self.engine_probed = true
        self.engine_exe, self.engine_missing = Engine.locate()
    end
    return self.engine_exe, self.engine_missing
end

--- The bokai converter, or `nil`. An add-on, not a dependency.
function KfxDeDRM:getConverter()
    if not self.converter_probed then
        self.converter_probed = true
        self.converter_exe = Convert.locate()
    end
    return self.converter_exe
end

--- Both probes again, for an install made without restarting KOReader.
function KfxDeDRM:reprobe()
    self.engine_probed = nil
    self.converter_probed = nil
end

--- The two switches, off whenever bokai is not there.
function KfxDeDRM:targets()
    return Convert.targets(self.cfg, self:getConverter())
end

function KfxDeDRM:engineMissingText(reason)
    local head = reason == Engine.NOT_INSTALLED
        and T(_("kfxdedrm is not installed.\nNothing found at %1."), Engine.BIN_DIR)
        or T(_("No kfxdedrm build runs on this Kindle.\nAll %1 builds in %2 failed to start."),
            #Engine.ABI_VARIANTS, Engine.BIN_DIR)
    local first = reason == Engine.NOT_INSTALLED and _("Download") or _("Re-download")
    return table.concat({
        head,
        "",
        T("1.  %1  %2", first, Engine.RELEASE_ASSET),
        T(_("     from  %1"), Engine.RELEASES_URL),
        "",
        T(_("2.  Unzip it onto the Kindle as  %1/"), Engine.EXTENSION_DIR),
        "",
        _("Or let this plugin fetch it: “Download and install kfxdedrm”."),
    }, "\n")
end

function KfxDeDRM:bokaiMissingText()
    return table.concat({
        T(_("bokai is not installed.\nNothing runs at %1."), Convert.BIN_PATH),
        "",
        T("1.  %1  %2", _("Download"), Convert.RELEASE_ASSET),
        T(_("     from  %1"), Convert.RELEASES_URL),
        "",
        T(_("2.  Unzip it onto the Kindle as  %1/"), Convert.EXTENSION_DIR),
        "",
        _("Or let this plugin fetch it: “Download and install bokai”."),
    }, "\n")
end

function KfxDeDRM:showEngineMissing(reason)
    UIManager:show(InfoMessage:new{ text = self:engineMissingText(reason) })
end

--------------------------------------------------------------------------------
-- Open with… on a single book
--------------------------------------------------------------------------------

function KfxDeDRM:registerDocumentRegistryAuxProvider()
    DocumentRegistry:addAuxProvider({
        provider_name = self.fullname,
        provider = self.name,
        order = 50, -- order in the OpenWith dialog
        -- No default-handler offer either way: associating a book or a file
        -- type with this would turn a stray tap in the file browser into a
        -- decrypt run.
        disable_file = true,
        disable_type = true,
    })
end

--- Which files the *Open with…* dialog offers this for: the extensions the
--- engine has a code path for, DRM or not. `KfxDeDRM:openFile` is where a
--- DRM-free one is turned away.
function KfxDeDRM:isFileTypeSupported(file)
    return Engine.formatOf(file) ~= nil
end

function KfxDeDRM:openFile(file)
    local format = Engine.formatOf(file)
    if not format then return end
    if not Scan.isEncrypted(file, format) then
        -- The engine copies whatever it receives into `Config.OUT_DIR`, so a
        -- DRM-free book would yield a second copy of itself. `lib/scan` gates
        -- the list on this; the file browser lists everything.
        UIManager:show(InfoMessage:new{
            text = _("This book carries no DRM."),
        })
        return
    end

    local stem = Engine.stemAndExtension(file)
    local asin = Scan.parseAsin(stem)
    self:decryptBooks({{
        path = file,
        format = format,
        title = Scan.titleFromStem(stem, asin),
        asin = asin,
        done = false,
    }})
end

--------------------------------------------------------------------------------
-- The book list
--------------------------------------------------------------------------------

function KfxDeDRM:scanBooks()
    return Scan.scan(self.cfg, self:targets())
end

--- `cfg.scan_dirs` as one line: the folder itself while there is one, a count
--- once naming them all would run off the screen.
function KfxDeDRM:foldersSummary()
    local dirs = self.cfg.scan_dirs
    if #dirs == 0 then
        return _("no folder")
    elseif #dirs == 1 then
        return dirs[1]
    end
    return T(_("%1 folders"), #dirs)
end

function KfxDeDRM:emptyText()
    if not Config.listsAnything(self.cfg) then
        return _("Nothing is being listed.\nPick a folder and a format in the plugin's menu.")
    elseif self.cfg.show_done then
        return T(_("No DRM'd books found.\nLooked in %1."), self:foldersSummary())
    end
    return _("Nothing left to decrypt.\nFinished books are hidden — turn that off in the plugin's menu.")
end

local function menu_items_for(books)
    local items = {}
    for i, book in ipairs(books) do
        items[i] = {
            text = book.done and ("✓ " .. book.title) or book.title,
            mandatory = util.getFriendlySize(book.size),
            book = book,
        }
    end
    return items
end

function KfxDeDRM:listSubtitle(books)
    return T(_("%1 to decrypt · into %2"), Scan.pending(books), Config.OUT_DIR)
end

function KfxDeDRM:showBooks()
    local exe, missing = self:getEngine()
    if not exe then
        return self:showEngineMissing(missing)
    end

    local books = self:scanBooks()
    if #books == 0 then
        UIManager:show(InfoMessage:new{ text = self:emptyText() })
        return
    end

    local menu
    menu = Menu:new{
        title = _("DRM'd books"),
        subtitle = self:listSubtitle(books),
        item_table = menu_items_for(books),
        covers_fullscreen = true,
        is_borderless = true,
        is_popout = false,
        title_bar_fm_style = true,
        onMenuSelect = function(this, item)
            self:decryptBooks({ item.book }, function()
                self:refreshList(this, item.book.path)
            end)
        end,
        close_callback = function()
            UIManager:close(menu)
            -- Decrypted books land in `Config.OUT_DIR`, which the browser may
            -- be sitting in.
            if self.fm_updated and self.ui and self.ui.onRefresh then
                self.fm_updated = nil
                self.ui:onRefresh()
            end
        end,
    }
    UIManager:show(menu)
end

--- The list again after a run, holding the page the acted-on book was on.
function KfxDeDRM:refreshList(menu, keep_path)
    local books = self:scanBooks()
    local items = menu_items_for(books)
    local itemnumber
    for i, item in ipairs(items) do
        if item.book.path == keep_path then
            itemnumber = i
            break
        end
    end
    menu:switchItemTable(nil, items, itemnumber, nil, self:listSubtitle(books))
end

--------------------------------------------------------------------------------
-- Running the engine
--------------------------------------------------------------------------------

--- The engine's output for `book` is already there.
local function decrypted(book)
    local out = Engine.outputPath(book.path, Config.OUT_DIR)
    return out ~= nil and exists(out)
end

--- `targets`'s steps over `out`, each run to its own exit.
---
--- Returns the `Convert` kinds that failed, none being the good case. A step
--- whose output is already there is skipped, so a book listed only for a
--- missing EPUB does not repack its KFX first.
function KfxDeDRM:convertOutputs(converter, targets, out, title)
    local failed = {}
    for _, step in ipairs(Convert.steps(targets, out)) do
        if not exists(step.output) then
            -- The EPUB step reads the KFX step's output; a failure upstream
            -- leaves nothing to open.
            if not exists(step.input) then
                logger.warn("kfxdedrm: no input at", step.input, "for", Convert.label(step.kind))
                table.insert(failed, step.kind)
            else
                Trapper:info(Convert.progress(step.kind) .. "\n\n" .. title, false, true)
                local ok = run(Convert.convertCommand(converter, step)) and exists(step.output)
                if not ok then
                    -- A half-written file reads as a finished one on the next
                    -- scan, and would be handed to the step after it as an
                    -- input.
                    if not remove_if_present(step.output) then
                        logger.warn("kfxdedrm: cannot remove", step.output)
                    end
                    table.insert(failed, step.kind)
                end
            end
        end
    end
    return failed
end

--- One book: the engine, then every conversion the settings ask for.
---
--- True once each of those outputs is there. A book whose decrypt is already
--- done still reaches here for the conversions alone.
function KfxDeDRM:runBook(exe, converter, targets, book)
    if not decrypted(book) then
        if not run(Engine.decryptCommand(exe, book.path, Config.OUT_DIR)) then
            return false
        end
        if not decrypted(book) then
            logger.warn("kfxdedrm: engine finished but wrote no file for", book.path)
            return false
        end
    end

    local out = Engine.outputPath(book.path, Config.OUT_DIR)
    if not converter or not out then return true end
    return #self:convertOutputs(converter, targets, out, book.title) == 0
end

--- What a finished run says. `left` counts books a stop skipped.
function KfxDeDRM.summary(done, failed, left)
    local head
    if done == 0 and failed == 0 then
        head = _("Nothing to do")
    elseif failed == 0 then
        head = T(_("Decrypted %1"), done)
    elseif done == 0 then
        head = T(_("All %1 failed"), failed)
    else
        head = T(_("Decrypted %1, %2 failed"), done, failed)
    end

    if left == 0 and failed == 0 then
        return head
    elseif left == 0 then
        return head .. "\n" .. _("See the log")
    elseif failed == 0 then
        return head .. "\n" .. T(_("Stopped, %1 left"), left)
    end
    return head .. "\n" .. T(_("Stopped, %1 left — see the log"), left)
end

--- One engine run per book, each followed by its conversions.
---
--- The banner between books is the only place a stop is read: both binaries run
--- to their own exit. Tapping it asks, and answering Stop ends the batch after
--- the book that is running.
function KfxDeDRM:decryptBooks(books, on_done)
    local exe, missing = self:getEngine()
    if not exe then
        return self:showEngineMissing(missing)
    end
    if #books == 0 then
        UIManager:show(InfoMessage:new{ text = self:emptyText() })
        return
    end

    local converter = self:getConverter()
    local targets = self:targets()
    local total = #books

    Trapper:wrap(function()
        Trapper:setPausedText(_("Stop after this book?"), _("Stop"), _("Continue"))

        local done, failed, left = 0, 0, 0
        for i, book in ipairs(books) do
            local banner = total == 1
                and T(_("Decrypting\n\n%1"), book.title)
                or T(_("Decrypting %1 of %2\n\n%3"), i, total, book.title)
            if not Trapper:info(banner) then
                left = total - (i - 1)
                break
            end
            if self:runBook(exe, converter, targets, book) then
                done = done + 1
            else
                failed = failed + 1
            end
        end

        Trapper:clear()
        self.fm_updated = done > 0
        UIManager:show(InfoMessage:new{ text = KfxDeDRM.summary(done, failed, left) })
        if on_done then on_done() end
    end)
end

function KfxDeDRM:decryptEverything()
    local exe, missing = self:getEngine()
    if not exe then
        return self:showEngineMissing(missing)
    end
    local pending = {}
    for _, book in ipairs(self:scanBooks()) do
        if not book.done then
            table.insert(pending, book)
        end
    end
    self:decryptBooks(pending)
end

--------------------------------------------------------------------------------
-- Fetching the two binaries
--------------------------------------------------------------------------------

--- Whether `source` has a working copy in place, by running it.
function KfxDeDRM:isInstalled(source)
    if source.key == "engine" then
        return (self:getEngine()) ~= nil
    end
    return self:getConverter() ~= nil
end

function KfxDeDRM:installRowText(key)
    local source = Install.source(key)
    if not self:isInstalled(source) then
        return T(_("Download and install %1"), source.name)
    end
    local tag = Install.installedTag(key)
    if tag then
        return T(_("Update %1 (%2 installed)"), source.name, tag)
    end
    -- Installed by hand, or by a copy of this plugin that did not record it.
    -- Neither binary reports a version, so there is nothing to name.
    return T(_("Update %1"), source.name)
end

--- Ask GitHub what is published, then `Install.run` if it is not what is here.
---
--- The whole thing blocks: the release list and the download are plain requests
--- and there is nothing to poll them against. The banner therefore reports
--- steps and offers no stop, which is why each `Trapper:info` here skips the
--- dismiss check -- a Pause box that could not take effect would only lie.
function KfxDeDRM:installOrUpdate(key)
    local source = Install.source(key)
    local NetworkMgr = require("ui/network/manager")

    NetworkMgr:runWhenOnline(function()
        Trapper:wrap(function()
            local function step(text)
                Trapper:info(text, false, true)
            end
            local function done(text)
                Trapper:clear()
                UIManager:show(InfoMessage:new{ text = text })
            end

            step(T(_("Asking GitHub about %1…"), source.name))
            local release, err = Install.available(source)
            if not release then
                return done(T(_("Could not read the release list for %1.\n%2"), source.name, err))
            end

            if Install.installedTag(key) == release.tag and self:isInstalled(source) then
                return done(T(_("%1 is up to date (%2)."), source.name, release.tag))
            end

            local tag, failure = Install.run(source, release, step)
            if not tag then
                return done(T(_("%1 was not installed.\n%2"), source.name, failure))
            end

            Install.rememberTag(key, tag)
            self:reprobe()
            done(T(_("%1 %2 installed."), source.name, tag))
        end)
    end)
end

--------------------------------------------------------------------------------
-- Settings
--------------------------------------------------------------------------------

function KfxDeDRM:saveConfig()
    Config.sanitized(self.cfg)
    Config.store(self.cfg)
end

function KfxDeDRM:isScanned(dir)
    for _, chosen in ipairs(self.cfg.scan_dirs) do
        if chosen == dir then return true end
    end
    return false
end

function KfxDeDRM:toggleScanned(dir)
    for i, chosen in ipairs(self.cfg.scan_dirs) do
        if chosen == dir then
            table.remove(self.cfg.scan_dirs, i)
            return self:saveConfig()
        end
    end
    table.insert(self.cfg.scan_dirs, dir)
    self:saveConfig()
end

--- One row per folder that holds a DRM'd book, plus every folder already
--- selected. Counting them stats each folder's entries, so this is built when
--- the submenu opens and not before.
function KfxDeDRM:folderItems()
    local items = {}
    for _, cand in ipairs(Scan.candidates(self.cfg)) do
        local dir = cand.dir
        items[#items + 1] = {
            text = T("%1  (%2)", Scan.folderLabel(dir), cand.books),
            checked_func = function() return self:isScanned(dir) end,
            callback = function() self:toggleScanned(dir) end,
        }
    end
    if #items == 0 then
        items[1] = {
            text = T(_("Nothing found under %1"), Config.DOCUMENTS_DIR),
            enabled = false,
        }
    end
    return items
end

function KfxDeDRM:alsoWriteItems()
    local items = {}
    if not self:getConverter() then
        items[1] = {
            text = _("bokai is not installed — where to get it"),
            keep_menu_open = true,
            callback = function()
                UIManager:show(InfoMessage:new{ text = self:bokaiMissingText() })
            end,
            separator = true,
        }
    end
    items[#items + 1] = {
        text = _("KFX — the .kfx-zip bundle as one container"),
        enabled_func = function() return self:getConverter() ~= nil end,
        checked_func = function() return self.cfg.pack_kfx end,
        callback = function()
            self.cfg.pack_kfx = not self.cfg.pack_kfx
            self:saveConfig()
        end,
    }
    items[#items + 1] = {
        text = _("EPUB — what KOReader can open"),
        enabled_func = function() return self:getConverter() ~= nil end,
        checked_func = function() return self.cfg.convert_epub end,
        callback = function()
            self.cfg.convert_epub = not self.cfg.convert_epub
            self:saveConfig()
        end,
    }
    return items
end

function KfxDeDRM:aboutText()
    local exe, missing = self:getEngine()
    local engine_line = exe and T(_("Engine: %1"), exe)
        or (missing == Engine.NOT_INSTALLED
            and T(_("Engine: not installed at %1"), Engine.BIN_DIR)
            or T(_("Engine: no build in %1 runs here"), Engine.BIN_DIR))
    local converter = self:getConverter()
    return table.concat({
        engine_line,
        converter and T(_("bokai: %1"), converter) or T(_("bokai: not installed at %1"), Convert.BIN_PATH),
        "",
        T(_("Decrypted books land in %1."), Config.OUT_DIR),
        T(_("Settings file: %1"), Config.PATH),
        "",
        _("The engine and bokai are separate installs, shared with the standalone kfxdedrm-fe app."),
    }, "\n")
end

function KfxDeDRM:addToMainMenu(menu_items)
    menu_items.kfxdedrm = {
        text = self.fullname,
        sorting_hint = "more_tools",
        sub_item_table = {
            {
                text = _("Books…"),
                callback = function() self:showBooks() end,
            },
            {
                text = _("Decrypt everything listed"),
                callback = function() self:decryptEverything() end,
                separator = true,
            },
            {
                text = _("Folders to scan"),
                sub_item_table_func = function() return self:folderItems() end,
            },
            {
                text = _("Formats to list"),
                sub_item_table = {
                    {
                        text = _("KFX"),
                        checked_func = function() return self.cfg.types_kfx end,
                        callback = function()
                            self.cfg.types_kfx = not self.cfg.types_kfx
                            self:saveConfig()
                        end,
                    },
                    {
                        text = _("MOBI family (azw3, azw4, mobi)"),
                        checked_func = function() return self.cfg.types_mobi end,
                        callback = function()
                            self.cfg.types_mobi = not self.cfg.types_mobi
                            self:saveConfig()
                        end,
                    },
                },
            },
            {
                text = _("Also write"),
                sub_item_table_func = function() return self:alsoWriteItems() end,
            },
            {
                text = _("Keep finished books listed"),
                checked_func = function() return self.cfg.show_done end,
                callback = function()
                    self.cfg.show_done = not self.cfg.show_done
                    self:saveConfig()
                end,
                separator = true,
            },
            {
                text_func = function() return self:installRowText("engine") end,
                callback = function() self:installOrUpdate("engine") end,
            },
            {
                text_func = function() return self:installRowText("bokai") end,
                callback = function() self:installOrUpdate("bokai") end,
                separator = true,
            },
            {
                text = _("Look for the engine again"),
                keep_menu_open = true,
                callback = function()
                    self:reprobe()
                    UIManager:show(InfoMessage:new{ text = self:aboutText() })
                end,
            },
            {
                text = _("Where things are"),
                keep_menu_open = true,
                callback = function()
                    UIManager:show(InfoMessage:new{ text = self:aboutText() })
                end,
            },
        },
    }
end

return KfxDeDRM
