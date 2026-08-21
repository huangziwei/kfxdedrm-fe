-- Install.unpack over the two archives GitHub actually serves, with libarchive
-- stood in for by `unzip`. What is checked is the layout that comes out.
local harness = require("harness")
local check, eq = harness.check, harness.eq

local Install = require("lib.install")
local lfs = require("libs/libkoreader-lfs")


local CACHE = harness.SPEC .. "/cache"
local OUT = CACHE .. "/unpacked"
os.execute("rm -rf '" .. OUT .. "'")

local engine = Install.source("engine")
local bokai = Install.source("bokai")

-- kfxdedrmmobi.zip: rooted at kfxdedrm/
local dest = OUT .. "/kfxdedrm"
local written, err = Install.unpack(CACHE .. "/kfxdedrmmobi.zip", engine.marker, dest)
check("the engine archive unpacks", written ~= nil, err)
-- Nine entries in the archive, two of which are the folders themselves.
eq("its seven files", written, 7)
eq("the four ABI builds land under bin/",
    lfs.attributes(dest .. "/bin/kfxdedrmhf_c11", "mode"), "file")
eq("and so does the launcher the engine ships",
    lfs.attributes(dest .. "/bin/run_cmd.sh", "mode"), "file")
eq("config.xml lands at the root", lfs.attributes(dest .. "/config.xml", "mode"), "file")
eq("the archive's own folder name is not repeated",
    lfs.attributes(dest .. "/kfxdedrm", "mode"), nil)
eq("a build comes out whole", lfs.attributes(dest .. "/bin/kfxdedrmhf_c11", "size"), 795072)

-- The engine's own bin/ is where Engine.locateIn looks.
local Engine = require("lib.engine")
local paths = Engine.variantPaths(dest .. "/bin")
local present = 0
for _, p in ipairs(paths) do
    if lfs.attributes(p, "mode") == "file" then present = present + 1 end
end
eq("every variant the probe walks is there", present, 4)

-- bokai's zip: rooted two deep, at extensions/bokai/
local bdest = OUT .. "/bokai"
local bwritten, berr = Install.unpack(CACHE .. "/bokai.zip", bokai.marker, bdest)
check("the bokai archive unpacks", bwritten ~= nil, berr)
eq("both of its files", bwritten, 2)
eq("the binary lands under bin/", lfs.attributes(bdest .. "/bin/bokai", "mode"), "file")
eq("at its published size", lfs.attributes(bdest .. "/bin/bokai", "size"), 5274348)
eq("the two folders above it are gone",
    lfs.attributes(bdest .. "/extensions", "mode"), nil)

-- An archive that is not the one asked for is refused before anything lands.
local wrong = OUT .. "/wrong"
local w, werr = Install.unpack(CACHE .. "/bokai.zip", engine.marker, wrong)
eq("an archive without the marker unpacks nothing", w, nil)
check("and says why", werr and werr:find(engine.marker, 1, true) ~= nil, werr)
eq("leaving no folder behind", lfs.attributes(wrong, "mode"), nil)

local missing, merr = Install.unpack(CACHE .. "/nonexistent.zip", engine.marker, OUT .. "/none")
eq("a file that is not an archive unpacks nothing", missing, nil)
check("and says so", merr ~= nil, merr)

os.execute("rm -rf '" .. OUT .. "'")
return harness.report()
