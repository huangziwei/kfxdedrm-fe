-- lib/install's decisions, against the three release lists GitHub actually
-- serves and the archives they actually publish.
local harness = require("harness")
local check, eq, eqlist = harness.check, harness.eq, harness.eqlist

local Install = require("lib.install")
local fixtures = require("fixtures.releases")


local engine = Install.source("engine")
local bokai = Install.source("bokai")
check("both sources are named", engine ~= nil and bokai ~= nil)
eq("an unknown key resolves to nothing", Install.source("nope"), nil)

-- This plugin is not one of the two: it is fetched on its own, into wherever
-- KOReader loaded it from.
local app = Install.appSource(harness.PLUGIN)
eq("and the plugin is not among them", Install.source(app.key), nil)

--------------------------------------------------------------------------------
-- pickRelease
--------------------------------------------------------------------------------

local tag, url, name, sha = Install.pickRelease(fixtures.dedrm, engine)
eq("the engine comes off the newest release carrying its asset", tag, "v10.0.30")
eq("and it is the mobi-capable asset", name, "kfxdedrmmobi.zip")
check("with a download url", url and url:find("kfxdedrmmobi.zip", 1, true) ~= nil, url)
eq("that release publishes no checksum", sha, nil)

-- /releases/latest returns the newest non-prerelease; none carries the engine.
local carrying, stable_with_asset = 0, 0
for _, r in ipairs(fixtures.dedrm) do
    local has = false
    for _, a in ipairs(r.assets) do
        if a.name:match(engine.asset) then has = true end
    end
    if has then
        carrying = carrying + 1
        check("a release carrying the engine is a prerelease: " .. r.tag_name, r.prerelease == true)
        if not r.prerelease then stable_with_asset = stable_with_asset + 1 end
    end
end
check("several releases carry it", carrying > 1, carrying)
eq("and none of them is what /releases/latest would return", stable_with_asset, 0)

-- bokai, taken by asset pattern.
local bver, burl, bname, bsha = Install.pickRelease(fixtures.sidle, bokai)
local expected_asset
for _, r in ipairs(fixtures.sidle) do
    for _, a in ipairs(r.assets) do
        if not expected_asset and a.name:match(bokai.asset) then
            expected_asset = a.name
        end
    end
end
eq("named by pattern, not by version", bname, expected_asset)
check("with a download url", burl ~= nil)
check("and that one publishes a checksum", bsha ~= nil and bsha:find(".sha256", 1, true) ~= nil, bsha)

-- `bokai-v0.1.3` tags a release whose asset carries `v0.1.3`; the sidle tag
-- `v0.1.9` carries `bokai-v0.1.2-kindle.zip`.
eq("bokai records the version its asset carries", bver, "v0.1.3")
local bundled = {}
for _, r in ipairs(fixtures.sidle) do
    if not r.tag_name:match("^bokai%-") then bundled[#bundled + 1] = r end
end
local sver, _, sname = Install.pickRelease(bundled, bokai)
eq("the sidle tag v0.1.9 carries bokai v0.1.2", sname, "bokai-v0.1.2-kindle.zip")
eq("and v0.1.2 is what is recorded", sver, "v0.1.2")
eq("an asset with no version falls back to the tag",
    bokai.version("bokai--kindle.zip", "v9.9.9"), "v9.9.9")
eq("the engine's asset never carries one",
    engine.version("kfxdedrmmobi.zip", "v10.0.30"), "v10.0.30")

-- `bokai-v9.9.9` carries no assets.
local half_published = { { tag_name = "bokai-v9.9.9", assets = {} } }
for _, r in ipairs(fixtures.sidle) do half_published[#half_published + 1] = r end
eq("a release with no assets yet is passed over",
    Install.pickRelease(half_published, bokai), bver)

eq("a list with nothing matching picks nothing", Install.pickRelease({
    { tag_name = "v1", assets = { { name = "source.zip", browser_download_url = "u" } } },
}, engine), nil)
eq("an empty list picks nothing", Install.pickRelease({}, engine), nil)
eq("a nil list picks nothing", Install.pickRelease(nil, engine), nil)
eq("a draft is skipped", Install.pickRelease({
    { tag_name = "v2", draft = true, assets = { { name = "kfxdedrmmobi.zip", browser_download_url = "u" } } },
}, engine), nil)

--------------------------------------------------------------------------------
-- This plugin's own release
--------------------------------------------------------------------------------

local aver, aurl, aname, asha = Install.pickRelease(fixtures.fe, app)
eq("the plugin comes off the newest release carrying its asset",
    aname, "kfxdedrm-koplugin-v0.4.0.zip")
eq("named by what the filename carries, not by the tag", aver, "v0.4.0")
check("with a download url", aurl ~= nil)
check("and a checksum beside it", asha ~= nil and asha:find(".sha256", 1, true) ~= nil, asha)

-- One release carries both frontends; each source takes only its own.
check("the standalone app's asset is not this one",
    not ("kfxdedrm-fe-v0.4.0-kindle.zip"):match(app.asset))
check("nor is the checksum beside its own",
    not ("kfxdedrm-koplugin-v0.4.0.zip.sha256"):match(app.asset))
check("nor is an add-on's", not ("bokai-v0.1.3-kindle.zip"):match(app.asset))

-- v0.2.0 and v0.1.0 predate the plugin, and are passed over for the tag alone.
local early = {}
for _, r in ipairs(fixtures.fe) do
    if r.tag_name == "v0.2.0" or r.tag_name == "v0.1.0" then early[#early + 1] = r end
end
eq("a release from before the plugin existed carries nothing for it",
    Install.pickRelease(early, app), nil)

-- The tag `v0.4.0` against the `0.4.0` [workspace.package] spells.
eq("a later release is offered", Install.isNewer("v0.5.0", "0.4.0"), true)
eq("and a build already at it is not", Install.isNewer("v0.4.0", "0.4.0"), false)
eq("nor is a downgrade", Install.isNewer("v0.3.0", "0.4.0"), false)
eq("whichever side carries the v", Install.isNewer("0.3.0", "v0.4.0"), false)
eq("a missing component reads as zero", Install.isNewer("v0.5", "0.5.0"), false)
eq("and the other way round", Install.isNewer("v0.5.0", "0.5"), false)
eq("a third component still counts", Install.isNewer("v0.5.1", "0.5"), true)
eq("a release beats its own candidate", Install.isNewer("v0.5.0", "0.5.0-rc1"), true)
eq("and the candidate does not beat it", Install.isNewer("v0.5.0-rc1", "0.5.0"), false)
eq("two candidates are not ordered", Install.isNewer("v0.5.0-rc2", "0.5.0-rc1"), false)
eq("a version that cannot be read starts nothing", Install.isNewer("nightly", "0.4.0"), false)
eq("nor an empty one", Install.isNewer("", "0.4.0"), false)
eq("nor a bare v", Install.isNewer("v", "0.4.0"), false)
eq("and nothing is newer than one", Install.isNewer("v99.0.0", "nightly"), false)
-- Wider than the u32 `install::selfupdate` reads a component into.
eq("nor is a component past what the other port can hold",
    Install.isNewer("v99999999999.0.0", "0.4.0"), false)

-- `_meta.lua` is what a copy KOReader is not holding reports.
local meta = harness.PLUGIN .. "/_meta.lua"
local version = Install.metaVersion(meta)
check("this copy names a version", version ~= nil and version ~= "", version)
eq("and it reads as one", Install.isNewer("v99.0.0", version), true)
eq("a file that is not there names none", Install.metaVersion(harness.SPEC .. "/nope.lua"), nil)

-- `build.sh` stamps both from [workspace.package]; `isNewer` compares the
-- release tag against this one.
local cargo = assert(io.open(harness.SPEC .. "/../../Cargo.toml", "r"))
local manifest = cargo:read("*all")
cargo:close()
eq("and it is the version Cargo.toml carries",
    version, manifest:match('%[workspace%.package%]%s*\nversion%s*=%s*"([^"]*)"'))

check("the plugin verifies against the folder it was loaded from", app.verify(harness.PLUGIN))
check("and not against one with no plugin in it", not app.verify(harness.SPEC))

--------------------------------------------------------------------------------
-- prefixFor, against the real archive listings
--------------------------------------------------------------------------------

local kfx_zip = {
    "kfxdedrm/", "kfxdedrm/bin/",
    "kfxdedrm/bin/kfxdedrmhf_c11", "kfxdedrm/bin/kfxdedrmhf_old",
    "kfxdedrm/bin/kfxdedrm_c11", "kfxdedrm/bin/kfxdedrm_old",
    "kfxdedrm/bin/run_cmd.sh", "kfxdedrm/config.xml", "kfxdedrm/menu.json",
}
local bokai_zip = {
    "extensions/bokai/", "extensions/bokai/bin/",
    "extensions/bokai/bin/bokai", "extensions/bokai/config.xml",
}
eq("kfxdedrmmobi.zip unpacks out of its own folder",
    Install.prefixFor(kfx_zip, engine.marker), "kfxdedrm/")
eq("bokai's zip unpacks out of a deeper one",
    Install.prefixFor(bokai_zip, bokai.marker), "extensions/bokai/")
local koplugin_zip = {
    "kfxdedrm.koplugin/", "kfxdedrm.koplugin/lib/",
    "kfxdedrm.koplugin/_meta.lua", "kfxdedrm.koplugin/main.lua",
    "kfxdedrm.koplugin/lib/install.lua", "LICENSE",
}
eq("the plugin's zip unpacks out of its own folder, LICENSE left behind",
    Install.prefixFor(koplugin_zip, app.marker), "kfxdedrm.koplugin/")
eq("an archive rooted on the marker has no prefix",
    Install.prefixFor({ "bin/bokai", "config.xml" }, bokai.marker), "")
eq("an archive without the marker is refused",
    Install.prefixFor({ "readme.txt" }, bokai.marker), nil)
-- The marker is matched as a path suffix, not anywhere in the name.
eq("a lookalike name is not the marker",
    Install.prefixFor({ "x/not-bin/bokai-old" }, bokai.marker), nil)

--------------------------------------------------------------------------------
-- digestFrom
--------------------------------------------------------------------------------

local d = string.rep("a", 64)
eq("a sha256sum line", Install.digestFrom(d .. "  bokai-v0.1.2-kindle.zip", "bokai-v0.1.2-kindle.zip"), d)
eq("with a binary marker", Install.digestFrom(d .. " *file.zip", "file.zip"), d)
eq("a full path in the line still matches the name",
    Install.digestFrom(d .. "  /tmp/build/file.zip", "file.zip"), d)
eq("a bare digest is taken as it is", Install.digestFrom(d .. "\n", "anything.zip"), d)
eq("uppercase is folded", Install.digestFrom(string.rep("A", 64) .. "  f.zip", "f.zip"), string.rep("a", 64))
eq("a short digest is not one", Install.digestFrom("abc  f.zip", "f.zip"), nil)
eq("no text, no digest", Install.digestFrom(nil, "f.zip"), nil)

--------------------------------------------------------------------------------
-- The install record, which native/ reads and writes too
--------------------------------------------------------------------------------

-- `native/tests/shared_settings_file.rs` holds the same fixture. The two
-- renderers agree byte for byte.
local fixture_path = harness.SPEC .. "/fixtures/installs.txt"
local fixture = assert(io.open(fixture_path, "r"))
local expected = fixture:read("*all")
fixture:close()

eq("the record renders the bytes native/ writes",
    Install.renderRecord({ engine = "v10.0.30", bokai = "v0.1.3" }), expected)

local read_back = Install.record(fixture_path)
eq("and reads its own file back", read_back.engine, "v10.0.30")
eq("both of them", read_back.bokai, "v0.1.3")
eq("a tag off the shared file", Install.installedTag("engine", fixture_path), "v10.0.30")
eq("and nothing for a key it does not name", Install.installedTag("nope", fixture_path), nil)

-- A file that is not there is an empty record.
eq("a missing file records nothing", next(Install.record(harness.SPEC .. "/cache/absent.txt")), nil)

local written = harness.SPEC .. "/cache/installs.txt"
os.remove(written)
Install.rememberTag("bokai", "v0.1.3", written)
Install.rememberTag("engine", "v10.0.30", written)
local round = assert(io.open(written, "r"))
eq("a record written a key at a time is the same file", round:read("*all"), expected)
round:close()
os.remove(written)

-- A hand-edited file costs only the lines that are wrong.
local hand = harness.SPEC .. "/cache/hand.txt"
local out = assert(io.open(hand, "w"))
out:write("# a comment\n\nengine = v10.0.30\nbokai =\nnonsense\n= orphan\n")
out:close()
local edited = Install.record(hand)
eq("a key with no value is no record", edited.bokai, nil)
eq("and the good line still reads", edited.engine, "v10.0.30")
os.remove(hand)

return harness.report()
