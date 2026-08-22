-- The Rust test suite, run against the Lua ports.
local harness = require("harness")
local check, eq, eqlist = harness.check, harness.eq, harness.eqlist

local Config = require("lib.config")
local Convert = require("lib.convert")
local Engine = require("lib.engine")
local Mobi = require("lib.mobi")
local Scan = require("lib.scan")


--------------------------------------------------------------------------------
-- engine
--------------------------------------------------------------------------------

eqlist("probe order matches the engine's own launcher",
    Engine.variantPaths("/x/bin"),
    { "/x/bin/kfxdedrmhf_c11", "/x/bin/kfxdedrmhf_old", "/x/bin/kfxdedrm_old", "/x/bin/kfxdedrm_c11" })

eq("kfx output takes a new extension",
    Engine.outputPath("/d/Items01/Book_B000O76ON6.kfx", "/mnt/us/dedrm"),
    "/mnt/us/dedrm/Book_B000O76ON6.kfx-zip")
eq("mobi keeps its own name",
    Engine.outputPath("/d/Some Book.azw3", "/mnt/us/dedrm"),
    "/mnt/us/dedrm/Some Book.azw3")
eq("a dotted title keeps everything but the real extension",
    Engine.outputPath("/d/All of Us_ Vol. 1_B00XST7S8C.kfx", "/o"),
    "/o/All of Us_ Vol. 1_B00XST7S8C.kfx-zip")

eq("classifies kfx", Engine.formatOf("a.kfx"), Engine.KFX)
for _, ext in ipairs(Engine.MOBI_EXTENSIONS) do
    eq("classifies " .. ext, Engine.formatOf("a." .. ext), Engine.MOBI)
end
eq("an upper-cased extension is the same extension", Engine.formatOf("a.AZW3"), Engine.MOBI)
eq("upper-cased kfx", Engine.formatOf("a.KFX"), Engine.KFX)
eq("azw is not a candidate", Engine.formatOf("a.azw"), nil)
eq("prc is not a candidate", Engine.formatOf("a.prc"), nil)
eq("the engine's own output is not a candidate", Engine.formatOf("a.kfx-zip"), nil)
eq("epub is not a candidate", Engine.formatOf("a.epub"), nil)
eq("a name with no extension is not a candidate", Engine.formatOf("noext"), nil)
eq("no output path without a format", Engine.outputPath("noext", "/o"), nil)
eq("a leading dot is all stem", (Engine.stemAndExtension("/d/.hidden")), ".hidden")
eq("a dot in a folder is not an extension", Engine.formatOf("/my.books/noext"), nil)
eq("join reduces the slashes between", Engine.join("/o/", "x"), "/o/x")

--------------------------------------------------------------------------------
-- config
--------------------------------------------------------------------------------

local function cfg_eq(a, b)
    if #a.scan_dirs ~= #b.scan_dirs then return false end
    for i = 1, #a.scan_dirs do
        if a.scan_dirs[i] ~= b.scan_dirs[i] then return false end
    end
    for _, k in ipairs({ "types_kfx", "types_mobi", "pack_kfx", "convert_epub", "show_done" }) do
        if a[k] ~= b[k] then return false end
    end
    return true
end

local round = {
    scan_dirs = { "/mnt/us/documents", "/mnt/us/documents/Sidle" },
    types_kfx = true, types_mobi = false,
    pack_kfx = true, convert_epub = true, show_done = false,
}
check("round trips through the file format", cfg_eq(Config.parse(Config.render(round)), round))
local d = Config.default()
check("the default is a fixed point of the round trip", cfg_eq(Config.parse(Config.render(d)), d))

local none = Config.default()
none.scan_dirs = {}
check("deselecting every folder survives the round trip", cfg_eq(Config.parse(Config.render(none)), none))
check("nothing is listed with no folder", not Config.listsAnything(none))
eqlist("a file naming no folder takes the default",
    Config.parse("types_kfx = true").scan_dirs, { Config.ITEMS01_DIR })

local tolerant = Config.parse(table.concat({
    "# a comment", "", "scan_dir = /mnt/us/documents", "types_kfx = maybe",
    "types_mobi", "nonsense = true", "convert_epub = 1", "show_done=OFF",
}, "\n"))
eqlist("a read folder", tolerant.scan_dirs, { Config.DOCUMENTS_DIR })
eq("tolerant of spelling and spacing", tolerant.show_done, false)
eq("convert_epub = 1 reads as on", tolerant.convert_epub, true)
eq("an absent key keeps its default", tolerant.pack_kfx, false)
eq("an unparseable value keeps its default", tolerant.types_kfx, true)
eq("a line with no = keeps its default", tolerant.types_mobi, true)

local refused = Config.parse("scan_dir = " .. Config.OUT_DIR .. "\nscan_dir = relative/path")
eq("a folder that could eat a book is refused", #refused.scan_dirs, 0)
check("and lists nothing", not Config.listsAnything(refused))

local twice = Config.parse(table.concat({
    "scan_dir = " .. Config.ITEMS01_DIR,
    "scan_dir = " .. Config.DOCUMENTS_DIR,
    "scan_dir = " .. Config.ITEMS01_DIR,
}, "\n"))
eqlist("a folder named twice is scanned once", twice.scan_dirs,
    { Config.ITEMS01_DIR, Config.DOCUMENTS_DIR })

eqlist("a fresh install reads the folder this firmware downloads into",
    Config.default().scan_dirs, { Config.ITEMS01_DIR })

local no_format = Config.default()
no_format.types_kfx, no_format.types_mobi = false, false
check("switching every format off is also an empty configuration", not Config.listsAnything(no_format))

-- Byte-for-byte against what `Config::render` wrote.
local function read_file(path)
    local f = io.open(path, "r")
    if not f then return nil end
    local s = f:read("*all")
    f:close()
    return s
end
local fixtures_dir = harness.SPEC .. "/fixtures/"
eq("render matches the Rust byte for byte (default)",
    Config.render(Config.default()), read_file(fixtures_dir .. "config-default.txt"))
local alt = { scan_dirs = {}, types_kfx = false, types_mobi = true,
    pack_kfx = true, convert_epub = true, show_done = false }
eq("render matches the Rust byte for byte (no folder)",
    Config.render(alt), read_file(fixtures_dir .. "config-no-folder.txt"))

--------------------------------------------------------------------------------
-- convert
--------------------------------------------------------------------------------

local BOTH = { kfx = true, epub = true }
local NEITHER = { kfx = false, epub = false }

local function kinds(steps)
    local out = {}
    for i, s in ipairs(steps) do out[i] = s.kind end
    return out
end

local no_converter = Convert.targets({ pack_kfx = true, convert_epub = true }, nil)
eq("nothing is planned without a converter (kfx)", no_converter.kfx, false)
eq("nothing is planned without a converter (epub)", no_converter.epub, false)
eq("and no steps", #Convert.steps(no_converter, "/o/Book.kfx-zip"), 0)

local steps = Convert.steps(BOTH, "/o/Book.kfx-zip")
eqlist("the epub comes off the kfx this run packs", kinds(steps), { Convert.KFX, Convert.EPUB })
eq("the kfx step reads the bundle", steps[1].input, "/o/Book.kfx-zip")
eq("the kfx step writes one container", steps[1].output, "/o/Book.kfx")
eq("the second step reads the first one's output", steps[2].input, steps[1].output)
eq("the epub step's output", steps[2].output, "/o/Book.epub")

local epub_only = Convert.steps({ kfx = false, epub = true }, "/o/Book.kfx-zip")
eqlist("an epub alone is read straight out of the bundle", kinds(epub_only), { Convert.EPUB })
eq("straight from the bundle", epub_only[1].input, "/o/Book.kfx-zip")

for _, name in ipairs({ "Some Book.azw3", "Some Book.mobi" }) do
    local mobi_steps = Convert.steps(BOTH, "/o/" .. name)
    eqlist("a mobi-family copy has no bundle to pack: " .. name, kinds(mobi_steps), { Convert.EPUB })
    eq("its epub keeps the name: " .. name, mobi_steps[1].output, "/o/Some Book.epub")
end

eq("a format bokai cannot read is left alone (azw4)", #Convert.steps(BOTH, "/o/Some Book.azw4"), 0)
eq("a format bokai cannot read is left alone (noext)", #Convert.steps(BOTH, "/o/noext"), 0)

local dotted = Convert.steps(BOTH, "/o/All of Us_ Vol. 1_B00XST7S8C.kfx-zip")
eq("a dotted title keeps everything but the real extension (kfx)",
    dotted[1].output, "/o/All of Us_ Vol. 1_B00XST7S8C.kfx")
eq("a dotted title keeps everything but the real extension (epub)",
    dotted[2].output, "/o/All of Us_ Vol. 1_B00XST7S8C.epub")

eqlist("an upper-cased extension is the same extension",
    kinds(Convert.steps(BOTH, "/o/Some Book.AZW3")), { Convert.EPUB })

eqlist("the outputs are the steps' own outputs",
    Convert.outputs(BOTH, "/o/Book.kfx-zip"), { "/o/Book.kfx", "/o/Book.epub" })
eq("no targets, no outputs", #Convert.outputs(NEITHER, "/o/Book.kfx-zip"), 0)

eq("a missing binary resolves to no converter", Convert.locateAt("/nonexistent/bokai"), nil)
eq("a missing directory resolves to no converter", Convert.locateIn("/nonexistent/bin"), nil)

eqlist("probe order puts the hard-float build first",
    Convert.variantPaths("/x/bin"), { "/x/bin/bokai", "/x/bin/bokai-armsf" })

-- A build under ABI whose `--version` exits `code`, standing in for one the
-- loader accepts (0) or refuses (anything else).
local ABI = harness.SPEC .. "/cache/abi"
os.execute("rm -rf '" .. ABI .. "'")
local function variant(name, code)
    os.execute("mkdir -p '" .. ABI .. "'")
    local path = ABI .. "/" .. name
    local f = assert(io.open(path, "w"))
    f:write("#!/bin/sh\nexit " .. code .. "\n")
    f:close()
    os.execute("chmod +x '" .. path .. "'")
    return path
end

-- What is left after unpacking a zip whose hard-float build a device cannot
-- start: the name `Convert.locate` used to look for is not there.
local armsf = variant("bokai-armsf", 0)
eq("a soft-float-only install still resolves", Convert.locateIn(ABI), armsf)
-- Both installed, which is every install: the probe, not the name, decides.
variant("bokai", 126)
eq("a build that will not start is passed over for one that will",
    Convert.locateIn(ABI), armsf)
local hf = variant("bokai", 0)
eq("and with both running, hard-float wins on order", Convert.locateIn(ABI), hf)
variant("bokai", 1)
variant("bokai-armsf", 1)
eq("a directory where nothing starts resolves to no converter", Convert.locateIn(ABI), nil)
os.execute("rm -rf '" .. ABI .. "'")

for _, kind in ipairs({ Convert.KFX, Convert.EPUB }) do
    check("each kind names itself in three places: " .. kind,
        #Convert.extension(kind) > 0 and #Convert.progress(kind) > 0 and #Convert.label(kind) > 0)
end
check("the two kinds differ", Convert.extension(Convert.KFX) ~= Convert.extension(Convert.EPUB)
    and Convert.progress(Convert.KFX) ~= Convert.progress(Convert.EPUB))

--------------------------------------------------------------------------------
-- fixtures
--------------------------------------------------------------------------------

local TMP = harness.SPEC .. "/cache/tree"
os.execute("rm -rf '" .. TMP .. "'")

local function be(n, width)
    local out = {}
    for i = width, 1, -1 do
        out[i] = string.char(n % 256)
        n = math.floor(n / 256)
    end
    return table.concat(out)
end

--- The 78-byte PalmDB header, one record-list entry, then a record 0 carrying
--- `enc` -- `native/tests/discovery.rs`'s fixture.
local function palmdb(type_creator, enc)
    local head = string.rep("\0", 60) .. type_creator .. string.rep("\0", 8)
        .. be(1, 2) -- record count
        .. be(86, 4) -- record 0 begins at 86
        .. string.rep("\0", 4) -- out to the 86-byte header
    return head .. string.rep("\0", 12) .. be(enc, 2)
end

local function mkdirp(path)
    os.execute("mkdir -p '" .. path .. "'")
    return path
end
local function write(path, bytes)
    local f = assert(io.open(path, "wb"))
    f:write(bytes)
    f:close()
end
local function kfx(dir, stem, voucher)
    write(dir .. "/" .. stem .. ".kfx", "kfx")
    if voucher then
        local assets = mkdirp(dir .. "/" .. stem .. ".sdr/assets")
        write(assets .. "/voucher", "v")
    end
end
local function mobi(dir, name, enc)
    write(dir .. "/" .. name, palmdb("BOOKMOBI", enc))
end

local function base_cfg()
    local c = Config.default()
    c.scan_dirs = {}
    return c
end
local function scan_one(root, out, c)
    return Scan.scanIn({ root }, c, NEITHER, out)
end

--------------------------------------------------------------------------------
-- mobi
--------------------------------------------------------------------------------

local mdir = mkdirp(TMP .. "/mobi_types")
for _, case in ipairs({ { 0, Mobi.NONE }, { 1, Mobi.LEGACY }, { 2, Mobi.MOBIPOCKET } }) do
    write(mdir .. "/t" .. case[1] .. ".mobi", palmdb("BOOKMOBI", case[1]))
    eq("reads encryption type " .. case[1], Mobi.fileEncryption(mdir .. "/t" .. case[1] .. ".mobi"), case[2])
end
check("type 0 is not DRM", not Mobi.isDrm(Mobi.NONE))
check("type 1 is DRM", Mobi.isDrm(Mobi.LEGACY))
check("type 2 is DRM", Mobi.isDrm(Mobi.MOBIPOCKET))
write(mdir .. "/topaz.azw", palmdb("TPZ3TPZ3", 2))
eq("a Topaz database reads as nothing", Mobi.fileEncryption(mdir .. "/topaz.azw"), nil)
write(mdir .. "/short.mobi", "too short")
eq("a truncated file reads as nothing", Mobi.fileEncryption(mdir .. "/short.mobi"), nil)
eq("an absent file reads as nothing", Mobi.fileEncryption(mdir .. "/nope.mobi"), nil)

--------------------------------------------------------------------------------
-- scan
--------------------------------------------------------------------------------

local items = mkdirp(TMP .. "/kfx/Items01")
local out = mkdirp(TMP .. "/kfx/out")
kfx(items, "Good Book_ Subtitle_B000O76ON6", true)
kfx(items, "Half Book_B000FC1BQK", false) -- still downloading
write(items .. "/._Good Book_ Subtitle_B000O76ON6.kfx", "x")

local found = scan_one(items, out, base_cfg())
eq("a kfx needs its voucher to be listed", #found, 1)
eq("its title", found[1] and found[1].title, "Good Book: Subtitle")
eq("its asin", found[1] and found[1].asin, "B000O76ON6")
eq("its format", found[1] and found[1].format, Engine.KFX)
eq("not done", found[1] and found[1].done, false)

local docs = mkdirp(TMP .. "/mobi/documents")
local mout = mkdirp(TMP .. "/mobi/out")
mobi(docs, "Purchased.azw3", 2)
mobi(docs, "Old DRM.mobi", 1)
mobi(docs, "My Own Book.azw3", 0)
write(docs .. "/Notes.epub", "not a mobi")
write(docs .. "/Topaz.azw", palmdb("TPZ3TPZ3", 2))

local sideloads = scan_one(docs, mout, base_cfg())
local titles = {}
for i, b in ipairs(sideloads) do titles[i] = b.title end
table.sort(titles)
eqlist("a drm-free sideload is never listed", titles, { "Old DRM", "Purchased" })

-- done, marked or hidden
local ditems = mkdirp(TMP .. "/done/Items01")
local dout = mkdirp(TMP .. "/done/out")
kfx(ditems, "Fresh_B01MXXZOEW", true)
kfx(ditems, "Done_B078H4RWP7", true)
write(dout .. "/Done_B078H4RWP7.kfx-zip", "z")

local marked = scan_one(ditems, dout, base_cfg())
eq("both books are listed while finished ones show", #marked, 2)
local done_count = 0
for _, b in ipairs(marked) do if b.done then done_count = done_count + 1 end end
eq("one of them is done", done_count, 1)
eq("pending counts the rest", Scan.pending(marked), 1)

local hide = base_cfg()
hide.show_done = false
eq("hiding finished books drops it", #scan_one(ditems, dout, hide), 1)

-- a book missing a conversion is not done yet
local conv_cfg = base_cfg()
local both_targets = { kfx = true, epub = true }
local half = Scan.scanIn({ ditems }, conv_cfg, both_targets, dout)
local still_pending = 0
for _, b in ipairs(half) do if not b.done then still_pending = still_pending + 1 end end
eq("a book missing a conversion is not done yet", still_pending, 2)
write(dout .. "/Done_B078H4RWP7.kfx", "k")
write(dout .. "/Done_B078H4RWP7.epub", "e")
local whole = Scan.scanIn({ ditems }, conv_cfg, both_targets, dout)
local now_done = 0
for _, b in ipairs(whole) do if b.done then now_done = now_done + 1 end end
eq("with every output there it is", now_done, 1)

-- format toggles
local kfx_only = base_cfg()
kfx_only.types_mobi = false
local mixed = mkdirp(TMP .. "/mixed")
kfx(mixed, "K_B000000001", true)
mobi(mixed, "M.azw3", 2)
eq("kfx only lists the kfx", #scan_one(mixed, out, kfx_only), 1)
local mobi_only = base_cfg()
mobi_only.types_kfx = false
eq("mobi only lists the mobi", #scan_one(mixed, out, mobi_only), 1)
eq("both list both", #scan_one(mixed, out, base_cfg()), 2)

-- one level deep
local deep = mkdirp(TMP .. "/deep")
mkdirp(deep .. "/updates")
kfx(deep, "Top_B000000002", true)
kfx(deep .. "/updates", "Nested_B000000003", true)
eq("the scan stops at one level", #scan_one(deep, out, base_cfg()), 1)

-- roots in order
local r1 = mkdirp(TMP .. "/roots/one")
local r2 = mkdirp(TMP .. "/roots/two")
kfx(r1, "First_B000000004", true)
kfx(r2, "Second_B000000005", true)
local ordered = Scan.scanIn({ r1, r2 }, base_cfg(), NEITHER, out)
eq("roots are listed in the order they were given", ordered[1].title, "First")
eq("and then the second", ordered[2].title, "Second")

eq("an absent root yields nothing rather than failing",
    #scan_one(TMP .. "/nonexistent", out, base_cfg()), 0)

-- candidates
local lib = mkdirp(TMP .. "/library")
local holds = mkdirp(lib .. "/Downloads/Items01")
mkdirp(lib .. "/Empty")
mkdirp(lib .. "/Book_B000000006.sdr/assets")
kfx(holds, "Bought_B000000007", true)
local cands = Scan.candidatesIn(lib, base_cfg())
local labels = {}
for i, c in ipairs(cands) do labels[i] = c.dir end
eqlist("a folder holding a drm'd book is offered and an empty one is not",
    labels, { lib .. "/Downloads/Items01" })
eq("and it counts what it holds", cands[1].books, 1)

local selected = base_cfg()
selected.scan_dirs = { lib .. "/Empty" }
local kept = Scan.candidatesIn(lib, selected)
local kept_dirs = {}
for i, c in ipairs(kept) do kept_dirs[i] = c.dir end
check("a selected folder stays on the list when it empties",
    (function()
        for _, d in ipairs(kept_dirs) do if d == lib .. "/Empty" then return true end end
        return false
    end)(), table.concat(kept_dirs, ", "))

--------------------------------------------------------------------------------
-- labels and titles
--------------------------------------------------------------------------------

eq("a folder under documents takes its relative path",
    Scan.folderLabel("/mnt/us/documents/Downloads/Items01"), "Downloads/Items01")
eq("documents itself takes its own name",
    Scan.folderLabel("/mnt/us/documents"), "documents")
eq("a folder outside keeps its leading slash",
    Scan.folderLabel("/mnt/base/x"), "/mnt/base/x")

eq("an asin is the trailing token", Scan.parseAsin("Book_B000O76ON6"), "B000O76ON6")
eq("a short token is not an asin", Scan.parseAsin("Book_B0001"), nil)
eq("a lowercase token is not an asin", Scan.parseAsin("Book_b000o76on6"), nil)
eq("a token not starting with B is not an asin", Scan.parseAsin("Book_A000O76ON6"), nil)
eq("no underscore, no asin", Scan.parseAsin("Book"), nil)
eq("the title drops the asin", Scan.titleFromStem("Book_ Sub_B000O76ON6", "B000O76ON6"), "Book: Sub")
eq("and restores the colon", Scan.titleFromStem("A_ B", nil), "A: B")

os.execute("rm -rf '" .. TMP .. "'")

return harness.report()
