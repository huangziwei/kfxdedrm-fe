local _ = require("gettext")
return {
    fullname = _("KFX DeDRM"),
    description = _([[Decrypts the Kindle's own KFX and MOBI-family downloads through the kfxdedrm engine, and converts them with bokai. Both are separate installs; the plugin runs neither if they are missing.]]),
}
