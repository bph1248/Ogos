# Ogos

A media-focused helper and automation tool for Windows.

Run `ogos --help` for usage or refer to the [reference](./REFERENCE.md) for configuration.

# Features

- Audio:
    - Manage default audio endpoint devices and sample rates.
    - Manage active Equalizer APO configs.
- Binds:
    - Enable global hotkeys and dynamic key/button maps.
- Display:
    - Set brightness.
    - Asus PG32UCDM: Enable pixel cleaning.
    - Nvidia: Configure color bit depth and dither state.
    - Nvidia: Clamp color space when switching display mode via a headless version of [novideo_srgb](https://github.com/bph1248/novideo_srgb).
- Games:
    - Automate common tasks when launching games, such as switching display mode or changing desktop resolution.
- Media browser:
    - Collate files and folders into a unified view.
    - Enable Discord Rich Presence when viewing media.
    - Integrate with [mpv](https://mpv.io/) and [ReShade](https://reshade.me/) to automate switching display mode, sample rate and tone mapping parameters when launching videos.
- Taskbar:
    - Manage taskbar visibility by monitoring cursor collisions against an invisible window or 'hitbox'.
- Window shift:
    - Periodically 'pixel-shift' desktop windows in an effort to mitigate OLED burn-in when viewing static content.

# Screenshots

<p align="center">
    <img src="assets/media_browser_grid.webp" width="33%">
    <img src="assets/media_browser_grid_menus.webp" width="33%">
    <img src="assets/media_browser_details.webp" width="33%">
</p>

_Note: This project is in active development and breaking changes may occur._
