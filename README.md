# kfxdedrm-fe

A frontend for [kfxdedrm](https://github.com/Satsuoni/DeDRM_tools) on a jailbroken Kindle. 

## Build

```
git clone https://github.com/huangziwei/kfxdedrm-fe && cd kfxdedrm-fe/
rustup target add armv7-unknown-linux-musleabihf
./build.sh
```

## Install

1. Download `kfxdedrmmobi.zip` from the [DeDRM_tools releases](https://github.com/Satsuoni/DeDRM_tools/releases) and unzip it to `/mnt/us/extensions/kfxdedrm/`. Without it, kfxdedrm-fe opens on a screen that says so and does nothing else.

2. Download and unzip the latest `kfxdedrm-fe-v<x.y.z>-kindle.zip` from the
[release page](https://github.com/huangziwei/kfxdedrm-fe/releases), then copy:

| from | to | notes |
|:--|:--|:-- |
| `extensions/kfxdedrm-fe/` | `/mnt/us/extensions/kfxdedrm-fe/` | this exact path — the launcher and the settings file are hardcoded to it |
| `documents/KFXDeDRM.sh` | `/mnt/us/documents/KFXDeDRM.sh` | or anywhere you store your scriptlets |

## Screenshot

<p align="center">
    <img src=".github/assets/decrypt-one.png" height="500" />
    <img src=".github/assets/decrypt-all.png" height="500" />
</p>