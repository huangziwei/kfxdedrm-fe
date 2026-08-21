-- lib/install's decisions, against the release lists GitHub actually serves
-- and the two archives it actually publishes.
local harness = require("harness")
local check, eq, eqlist = harness.check, harness.eq, harness.eqlist

local Install = require("lib.install")
local fixtures = require("fixtures.releases")


local engine = Install.source("engine")
local bokai = Install.source("bokai")
check("both sources are named", engine ~= nil and bokai ~= nil)
eq("an unknown key resolves to nothing", Install.source("nope"), nil)

--------------------------------------------------------------------------------
-- pickRelease
--------------------------------------------------------------------------------

local tag, url, name, sha = Install.pickRelease(fixtures.dedrm, engine)
eq("the engine comes off the newest release carrying its asset", tag, "v10.0.30")
eq("and it is the mobi-capable asset", name, "kfxdedrmmobi.zip")
check("with a download url", url and url:find("kfxdedrmmobi.zip", 1, true) ~= nil, url)
eq("that release publishes no checksum", sha, nil)

-- Why /releases/latest is the wrong endpoint: it returns the newest release
-- that is not a prerelease, and no such release has ever carried the engine.
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

-- bokai: whichever release is newest at the time, taken by asset pattern.
local btag, burl, bname, bsha = Install.pickRelease(fixtures.sidle, bokai)
local expected_tag, expected_asset
for _, r in ipairs(fixtures.sidle) do
    for _, a in ipairs(r.assets) do
        if not expected_tag and a.name:match(bokai.asset) then
            expected_tag, expected_asset = r.tag_name, a.name
        end
    end
end
eq("bokai comes off the newest release carrying its asset", btag, expected_tag)
eq("named by pattern, not by version", bname, expected_asset)
check("with a download url", burl ~= nil)
check("and that one publishes a checksum", bsha ~= nil and bsha:find(".sha256", 1, true) ~= nil, bsha)
-- A tag can be published before its assets finish uploading; such a release
-- must not be picked over the last one that is whole.
local half_published = { { tag_name = "bokai-v9.9.9", assets = {} } }
for _, r in ipairs(fixtures.sidle) do half_published[#half_published + 1] = r end
eq("a release with no assets yet is passed over",
    Install.pickRelease(half_published, bokai), expected_tag)

eq("a list with nothing matching picks nothing", Install.pickRelease({
    { tag_name = "v1", assets = { { name = "source.zip", browser_download_url = "u" } } },
}, engine), nil)
eq("an empty list picks nothing", Install.pickRelease({}, engine), nil)
eq("a nil list picks nothing", Install.pickRelease(nil, engine), nil)
eq("a draft is skipped", Install.pickRelease({
    { tag_name = "v2", draft = true, assets = { { name = "kfxdedrmmobi.zip", browser_download_url = "u" } } },
}, engine), nil)

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

-- `native/tests/shared_settings_file.rs` holds the same fixture from the other
-- side. The two renderers have to agree byte for byte, or each frontend
-- rewrites the other's file and re-fetches what is already there.
local fixture_path = harness.SPEC .. "/fixtures/installs.txt"
local fixture = assert(io.open(fixture_path, "r"))
local expected = fixture:read("*all")
fixture:close()

eq("the record renders the bytes native/ writes",
    Install.renderRecord({ engine = "v10.0.30", bokai = "bokai-v0.1.3" }), expected)

local read_back = Install.record(fixture_path)
eq("and reads its own file back", read_back.engine, "v10.0.30")
eq("both of them", read_back.bokai, "bokai-v0.1.3")
eq("a tag off the shared file", Install.installedTag("engine", fixture_path), "v10.0.30")
eq("and nothing for a key it does not name", Install.installedTag("nope", fixture_path), nil)

-- A file that is not there is an empty record, not an error: it is what a
-- device carries until one of the two frontends fetches something.
eq("a missing file records nothing", next(Install.record(harness.SPEC .. "/cache/absent.txt")), nil)

local written = harness.SPEC .. "/cache/installs.txt"
os.remove(written)
Install.rememberTag("bokai", "bokai-v0.1.3", written)
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
