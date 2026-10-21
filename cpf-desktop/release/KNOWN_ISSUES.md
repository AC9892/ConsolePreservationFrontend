# Known issues

- This alpha does not claim full DSi compatibility. Built in applications can freeze, white screen, or return to the boot screen.
- Network dependent system features have not been fully tested.
- A clean Windows installation without WebView2 has not been tested.
- This local alpha executable is unsigned. Windows may show **Unknown Publisher** or an Open File security warning, especially when launched from a downloaded or temporary location. Code signing is still needed for a trusted public publisher identity.
- Nintendo DSi Sound has previously shown performance and audio problems; no full public regression run is complete.
- The first run importer validates expected sizes and a plain MBR or no$gba DSi footer. These checks do not establish that all dumps belong to the same console or guarantee successful boot.
- Virtual cartridge support is experimental. The separate game mounter is not part of this package.
