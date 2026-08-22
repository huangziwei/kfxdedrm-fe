local _ = require("gettext")
return {
    fullname = _("KFX DeDRM"),
    -- KOReader reads nothing from this field. It is here so `Where things are`
    -- can name the build a bug came from, and `build.sh` writes it from
    -- [workspace.package] in Cargo.toml -- do not edit it here.
    version = "0.5.0+dev",
    description = _([[Decrypts the Kindle's own KFX and MOBI-family downloads through the kfxdedrm engine, and converts them with bokai. Both are separate installs, which the plugin can fetch and update from their own GitHub releases.]]),
}
