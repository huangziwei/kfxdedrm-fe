--[[--
PalmDOC `encryption_type` for `.azw`, `.azw3`, `.azw4`, `.mobi`, `.prc`.

`lib/scan` gates a MOBI-family book on this. A Topaz database fails the
`BOOKMOBI` check and reads as `nil`.

`Mobi.isDrm` covers types 1 and 2, the pair the engine decodes.

A port of `native/src/mobi.rs`.
]]

local Mobi = {}

--- PalmDB `type`+`creator` at offset 60 for a Mobipocket-family database.
local BOOKMOBI = "BOOKMOBI"

--- Offset of the `type`+`creator` pair in the PalmDB header.
local TYPE_CREATOR_OFF = 60
--- Record-info list: 8 bytes per record, the first four its file offset.
local RECORD_LIST_OFF = 78
--- Header bytes through the first record-list entry's 4-byte offset.
local HEADER_PREFIX = RECORD_LIST_OFF + 4

--- `encryption_type` within record 0, and the bytes of record 0 to read.
local ENCRYPTION_OFF = 12
local REC0_PREFIX = ENCRYPTION_OFF + 2

--- What record 0's `encryption_type` field says. Any value but these three is
--- one the engine reports "Cannot decode unknown Mobipocket encryption type"
--- for.
Mobi.NONE = 0
Mobi.LEGACY = 1
Mobi.MOBIPOCKET = 2

--- True for `Mobi.LEGACY` and `Mobi.MOBIPOCKET`, the two the engine decodes.
function Mobi.isDrm(encryption)
    return encryption == Mobi.LEGACY or encryption == Mobi.MOBIPOCKET
end

--- Big-endian `u16` at 0-based `off`, or `nil` if the string is too short.
local function be16(bytes, off)
    local a, b = bytes:byte(off + 1, off + 2)
    if not b then return nil end
    return a * 256 + b
end

--- Big-endian `u32` at 0-based `off`, or `nil` if the string is too short.
local function be32(bytes, off)
    local a, b, c, d = bytes:byte(off + 1, off + 4)
    if not d then return nil end
    return ((a * 256 + b) * 256 + c) * 256 + d
end

--- Offset of record 0, from at least `HEADER_PREFIX` bytes. `nil` for a short
--- string or a type/creator pair that is not `BOOKMOBI`.
function Mobi.record0Offset(header)
    if header:sub(TYPE_CREATOR_OFF + 1, TYPE_CREATOR_OFF + 8) ~= BOOKMOBI then
        return nil
    end
    return be32(header, RECORD_LIST_OFF)
end

--- `encryption_type`, from at least `REC0_PREFIX` bytes of record 0.
function Mobi.encryptionOfRecord0(rec0)
    return be16(rec0, ENCRYPTION_OFF)
end

--- `Mobi.record0Offset` then `Mobi.encryptionOfRecord0` over one string
--- holding the header and record 0.
function Mobi.encryption(bytes)
    local rec0 = Mobi.record0Offset(bytes)
    if not rec0 then return nil end
    return Mobi.encryptionOfRecord0(bytes:sub(rec0 + 1))
end

--- `Mobi.encryption` over `path`, as two reads of `HEADER_PREFIX` and
--- `REC0_PREFIX` bytes. `nil` on an I/O error, a truncated file, or a
--- non-`BOOKMOBI` database.
function Mobi.fileEncryption(path)
    local f = io.open(path, "rb")
    if not f then return nil end

    local header = f:read(HEADER_PREFIX)
    if not header or #header < HEADER_PREFIX then
        f:close()
        return nil
    end
    local rec0 = Mobi.record0Offset(header)
    if not rec0 then
        f:close()
        return nil
    end

    if not f:seek("set", rec0) then
        f:close()
        return nil
    end
    local prefix = f:read(REC0_PREFIX)
    f:close()
    if not prefix or #prefix < REC0_PREFIX then return nil end
    return Mobi.encryptionOfRecord0(prefix)
end

--- `Mobi.isDrm` of `Mobi.fileEncryption`. False when that is `nil`.
function Mobi.isEncrypted(path)
    return Mobi.isDrm(Mobi.fileEncryption(path))
end

return Mobi
