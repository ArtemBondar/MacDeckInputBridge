# Steam library artwork

`assets/steam/` contains artwork for a local MacDeck shortcut:

| File | Steam use | Size |
| --- | --- | --- |
| `shortcut-icon.png` | shortcut icon | 256 × 256 |
| `library-capsule.png` | vertical card/grid artwork | 600 × 900 |
| `library-header.png` | horizontal library header | 920 × 430 |
| `library-hero.png` | wide library background/banner | 3840 × 1240 |

For a non-Steam shortcut, Steam stores artwork in the local user's library and
does not offer a supported import API. In Steam Desktop Mode, open **Library**,
right-click **MacDeck**, then use **Manage** / the artwork context menu to set
each custom image from this folder. The shortcut icon is already installed for
the Plasma launcher.

Steam store apps use these artwork types automatically through Steamworks. This
local app ships the same files for manual assignment only.
