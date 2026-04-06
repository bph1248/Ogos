# ToC

- [Config overview](#config-overview)
- [Config layout](#config-layout)
- [Keys & buttons](#keys--buttons)
- [Macros](#macros)
- [Tasks](#tasks)

# Config overview

#### Apps

A list of paths to apps Ogos depends on. If a path is not specified and a default path does not exist, the PATH environment variable will be searched instead.

- `epic`: [Epic Games Launcher](https://store.epicgames.com/en-US/download) (see [Games](#games)).
- `ffprobe`: [Ffprobe](https://ffmpeg.org/ffprobe.html) (see [Mpv](#mpv)).
- `gog`: [GOG GALAXY](https://www.gogalaxy.com/en/) (see [Games](#games)).
- `mpv`: [mpv](https://mpv.io/) (see [Mpv](#mpv)).
- `novideo_srgb`: [novideo_srgb](https://github.com/ledoge/novideo_srgb) (see [Display modes](#display-modes)).
- `skif`: [Special K Injection Frontend](https://www.special-k.info/) (see [Games](#games)).
- `steam`: [Steam](https://store.steampowered.com/about/) (see [Games](#games)).

#### Audio

Settings used in conjunction with `ogos --endpoint` / `--eq`.

- `endpoint_apps`: A list of named apps, such that `app args` is invoked after `ogos --endpoint <name>` is called.
- `eq_apo`
    - `master_config_path`: The path to the master Equalizer APO config file.
    - `custom_config_paths`: A list of named paths to Equalizer APO config files, such that `<path>` is copied to `master_config_path` when `ogos --eq <name>` is called.

#### Binds

Settings related to global hotkeys and context-aware key/button maps, used in conjunction with `ogos --binds` (see [Keys & Buttons](#keys--buttons), [Tasks](#tasks)).

- `hotkeys`
    - `prefix`: A list of modifier keys that must be held before pressing a trigger. Must contain at least one entry.
    - `triggers`: A list of keys mapped to tasks, such that a task is run if all keys (prefix + trigger) are pressed and then released.
- `maps`: A list of process names mapped to key/button maps, such that key/button maps activate only when the foreground window belongs to `<exe>`.
- `underscore`: A hotkey for typing an underscore.
- `qmk`: Settings used to assign key/button maps via the QMK/VIA API. Requires a QMK compatible keyboard.
    - `layer`: The keyboard layer to target.
    - `layout_path`: A path to a keyboard layout file, used to identify key column/row locations. Exported from [Keychron Launcher](https://launcher.keychron.com/#/keymap).

#### Discord

- `app_ids`: A list of named Discord app IDs, used to begin Rich Presence activities when launching a game or video. The names used here and by the [media browser](#media-browser) do not reflect the name displayed in Discord when an activity goes live - that is set when the application is created.
- `display_kind`: The text to display in Discord's member list when an activity goes live.

#### Display modes

Nvidia-specific graphics settings, customizable per display mode.

- `novideo_srgb`: Enable novideo_srgb's color space / gamma clamp. All operations occur in SDR mode, before switching to, and after switching from, HDR mode. (It's still possible to leave the clamp enabled in HDR mode, hence why this setting has a HDR variant.)

#### Games

Settings used in conjunction with `ogos --game <name>`, where `<name>` is an arbitrary reference to the following settings:

- `proc`: The name of the process to monitor. Any settings applied during launch are reverted when this process terminates.
- `url`: A launcher-specific game identifier, where the launcher is inferred from the URL. If specified, the game is launched via this launcher, else `proc` is called directly. Valid urls are:
    - `com.epicgames.launcher://`
    - `gog://`
    - `steam://`
- `args`: A list of args to pass to the game.
- `cursor_size`: The cursor size to set on on launch.
- `res`: The desktop resolution to set on launch.

#### Media browser

A GUI to collate files and folders into a unified view. Launch videos with `mpv` or invoke a file's default handler.

- `dirs`: A list of directories to collate.
- `window_inner_size`: The size of the window, excluding the border.
- `grid_cell_width`: The width of 'grid view' image cells, rounded to the next multiple of 2. Images are resized using Blackman filtering to fit within these cells, maintaining aspect ratio. Cell aspect ratio is 2:3.
- `details_cell_width`: Same as above but for 'details view'.
- `scroll_multiplier`: Adjusts scroll speed.

Right click a grid entry to add an image. Alternatively, place an image of the same name (sans extension) in `./images`.

Add/remove tags by right clicking on a grid entry. Move the cursor to the left edge of the window to reveal the tag list, whereby you can rename/remove tags and filter the current view.

Left click a grid entry to enter 'details view' and browse its contents - either  a list of files and folders (if the entry was a folder) or a single file (if the entry was a file). Right click within this view to access additional settings:

- __Maintain sample rate__: Prevent setting the default audio endpoint sample rate to match video metadata.
- __Override GLSL shaders__: Pass `override_glsl_shaders` to mpv (see [Mpv](#mpv)).
- __Discord Rich Presence__: Begin a Rich Presence activity on launch. Details of the activity depend on the type of media selected (can be overridden):
    - __TV__: If browsing a folder, use the folder name for __Details__ and the media's file name for __State__, else use the media's file name for __Details__.
    - __Movie__/__Words__: Use the media's file name for __Details__.

Press `back_button` or `esc` to the return to the previous view.

Potential future improvements to the media browser include:
- Dynamic image loading / memory management. Currently all images are loaded up front.
- Incorporating functionality from `--game`.

#### Mpv

Settings used in conjunction with `ogos [PATH]` and the media browser. [ffprobe](#apps) is required to read video metadata.

- `sdr_profile`: The profile name to forward to mpv when launching SDR videos. SDR videos are considered anything not HDR.
- `hdr_profile`: Same as above but for HDR videos. HDR videos are considered anything targeting PQ or HLG transfer functions.
- `reshade`: Inject ReShade and configure Lilium's static tone mapper for use with HDR videos. Video metadata must contain the max luminance property, else ReShade is disabled. Currently only mpv configured for `gpu-api=vulkan` is supported.
    - `profile`: Overrides `hdr_profile`.
    - `layer_path`: The path to ReShade's Vulkan layer manifest, used to disable ReShade for SDR and non-statically tone mapped videos.
    - `preset_path`: A path to a ReShade preset file. If video metadata contains the max luminance property then the value is written to `[lilium__tone_mapping.fx] InputLuminanceMax` on launch.
    - `settings_path`: A path to a ReShade settings file. A symlink to this file is created in mpv's directory on launch to allow ReShade to function. If the file cannot be symlinked (developer mode is not enabled), it will be copied.
- `default_glsl_shaders`: A list of GLSL shaders to forward to mpv. Follows the same format as mpv's `--glsl-shaders`.
- `override_glsl_shaders`: Overrides `default_glsl_shaders` (see [Media Browser](#media-browser)).

#### Pixel cleaning

Asus PG32UCDM only. Whether to run `let_walk_away` before signaling the monitor to begin pixel cleaning.

#### Taskbar

Settings used in conjunction with `ogos --taskbar`.

- `hitbox_entry`: The state of the hitbox when the taskbar is hidden. The hitbox is positioned largely offscreen save for an area afforded by `inset_px`, allowing the hitbox to overlap the desktop. Cursor collisions against the hitbox reveal the taskbar and move the hitbox to the 'exit' position. If the hitbox and the taskbar are positioned on different sides of the screen, the cursor will also snap to the taskbar. Elevated privileges are recommended to be able to snap the cursor when the foreground window belongs to a process of higher integrity.
    - `side`: The side of the screen on which to place the hitbox.
    - `inset_px`: The number of pixels by which the hitbox will overlap the desktop.
    - `cursor_snap_offset_px`: The number of pixels, offset from the start menu and parallel to the taskbar, used to calculate the position to which the cursor will snap when it collides with the hitbox.
- `hitbox_exit`: The state of the hitbox when the taskbar is visible. The hitbox is positioned to cover the majority of the screen save for an area occupied by the taskbar and any additional space afforded by `taskbar_offset_px`. Cursor collisions against the hitbox hide the taskbar and move the hitbox to the 'entry' position.
    - `taskbar_offset_px`: The number of pixels by which to offset the position of the hitbox, such that it creates a space between itself and the taskbar.
    - `jump_list_offset_px`: The number of pixels by which to offset the position of the hitbox, such that it creates a space between itself and any newly created jmplist.
    - `cursor_snap_offset_pc`: A percentage of the height of the screen (top down), used to calculate the vertical position to which the cursor will snap when it collides with the hitbox.

The hitbox is disabled if the foreground window is full screen.

#### Window shift

Settings used in conjunction with `ogos --window-shift`. Elevated privileges are recommended to be able to shift windows belonging to processes of higher integrity.

- `enable_immersive_dark_mode`: Enable dark mode for window title bars that otherwise don't support it.
- `interval_s`: The shift interval in seconds.
- `leeway_px`: The distance in pixels a window may stray from its 'anchor'. An anchor is the initial layout (position/dimensions) of a window. Moving or resizing a window resets this anchor to the new layout (provided an anchor constraint is not in effect).
- `stride_px`: The number of pixels to shift a window along the axes of the screen. The direction chosen is random.

Shift behavior can be customized with constraints. When the properties of a window belonging to `<exe>` matches the criteria of a constraint, the constraint is applied to the window.

- `constraints`
    - `anchor`: Define a window's anchor rather than infer it from the window's initial layout.
    - `attributes`: Manage window borders and round corners.
    - `center`: Center a window on screen.
    - `shift`: Disable shift.
<br>
- `criteria`
    - `relation`
        - `this`: Match criteria against the currently enumerated window.
        - `top_level_*`: Search for a top level window belonging to `<exe>` and match against that instead. The constraint still applies to the currently enumerated window.
    - `against`: The window property against which to match `text`. Multiple strings may be provided; only one needs to match.
    - `op`: The operation used in matching `text`.
        - `equals`: `text` must equal the window property.
        - `contains`: `text` may be a substring of the window property.

Shift is disabled if the foreground window is full screen or `left_button`/`left_ctrl` is held.

# Config layout

```
(
    // Optional
    apps: (
        // Optional, default: "C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe"
        epic: "<path>",
        // Optional
        ffprobe: "<path>",
        // Optional, default: "C:\Program Files (x86)\GOG Galaxy\GalaxyClient.exe"
        gog: "<path>",
        // Optional
        mpv: "<path>",
        // Optional, default: "./novideo_srgb/novideo_srgb.dll"
        novideo_srgb: "<path>",
        // Optional
        skif: "<path>",
        // Optional, default: "C:\Program Files (x86)\steam\steam.exe"
        steam: "<path>"
    ),
    // Optional
    audio: (
        // Optional
        endpoint_apps: {
            "<name>": (app: "<path>", args: ["<arg>", ..]),
            ..
        },
        // Optional
        eq_apo: (
            // Optional, default: "C:\Program Files\EqualizerAPO\config\config.txt"
            master_config_path: "<path>",
            custom_config_paths: {
                "<name>": "<path>",
                ..
            }
        )
    ),
    // Optional
    binds: (
        // Optional
        hotkeys: (
            prefix: [<*shift, *ctrl, *win, *alt>, ..],
            tasks: {
                <key>: <begin_pixel_cleaning, go_to_sleep, let_walk_away, print_window_info, toggle_display_mode>,
                ..
            }
        ),
        // Optional
        maps: {
            "<exe>": {
                <
                    <<key, button>: <key, button>>,
                    <macro>
                >,
                ..
            },
            ..
        },
        // Optional
        underscore: (
            prefix: <key>,
            trigger: <key>
        ),
        // Optional
        qmk: (
            layer: <uint>,
            layout_path: "<path>"
        )
    ),
    // Optional
    discord: (
        app_ids: (
            // Optional
            movies: "<id>",
            // Optional
            tv: "<id>",
            // Optional
            words: "<id>"
        ),
        // Optional, default: app_name
        display_kind: <app_name, state, details>
    ),
    // Optional
    display_modes: (
        sdr: (
            color_bit_depth: <default, 6, 8, 10, 12, 16>,
            dither: (
                bit_depth: <6, 8, 10>,
                state: <default, enabled, disabled>,
                mode: <spatial_static, spatial_static2x2, spatial_dynamic, spatial_dynamic2x2, temporal>
            ),
            // Optional
            novideo_srgb: (
                primaries: <edid, profile(path: "<path>")>,
                color_space_target: <bt_709, display_p3, adobe_rgb, bt_2020>,
                // Optional
                gamma: <
                    srgb,
                    bt_1886,
                    lstar,
                    custom(value: <float>, black_output_offset: <float>, intent: <absolute, relative>)
                >,
                enable_optimization: <bool>
            )
        ),
        hdr: (..)
    ),
    // Optional
    games: {
        "<name>": (
            proc: "<exe>",
            // Optional
            url: "<url>",
            // Optional
            args: ["<arg>", ..],
            // Optional
            cursor_size: <uint>,
            // Optional
            res: (<uint>, <uint>)
        ),
        ..
    },
    // Optional
    media_browser: (
        dirs: ["<path>", ..],
        // Optional
        window_inner_size: (<uint>, <uint>),
        grid_cell_width: <uint>,
        details_cell_width: <uint>,
        // Optional
        scroll_multiplier: <float>
    ),
    // Optional
    mpv: (
        sdr_profile: "<profile>",
        hdr_profile: "<profile>",
        // Optional
        default_glsl_shaders: "<shaders>",
        // Optional
        override_glsl_shaders: "<shaders>",
        // Optional
        reshade: (
            profile: "<profile>",
            // Optional, default: "C:\ProgramData\ReShade\ReShade64.json"
            layer_path: "<path>",
            preset_path: "<path>",
            settings_path: "<path>"
        )
    ),
    // Optional
    pixel_cleaning: (
        let_walk_away: <bool>
    ),
    // Optional
    taskbar: (
        hitbox_entry: (
            // Optional
            side: <left, top, right, bottom>,
            // Optional, default: 10
            inset_px: <uint>,
            // Optional
            cursor_snap_offset_px: <int>
        ),
        hitbox_exit: (
            // Optional, default: 100
            taskbar_offset_px: <uint>,
            // Optional, default: 60
            jump_list_offset_px: <uint>,
            // Optional
            cursor_snap_offset_pc: <uint>
        )
    ),
    // Optional
    window_shift: (
        // Optional
        enable_immersive_dark_mode: <bool>,
        interval_s: <uint>,
        leeway_px: <uint>,
        // Optional, default: (1, 1)
        stride_px: (<uint>, <uint>),
        constraints: {
            "<exe>": (
                // Optional
                anchor: (
                    criteria: (
                        // Optional, default: this
                        relation: <this, top_level_free, top_level_owned>,
                        against: <caption, class>,
                        text: ["<text>", ..],
                        op: <equals, contains>
                    ),
                    relative: (left: <int>, top: <int>, right: <int>, bottom: <int>)
                ),
                // Optional
                attributes: (
                    criteria: (..),
                    // Optional
                    disable_border: <bool>,
                    // Optional
                    disable_round_corners: <bool>
                ),
                // Optional
                center: (
                    criteria: (..)
                ),
                // Optional
                shift: (
                    criteria: (..)
                )
            ),
            ..
        }
    )
)
```

# Keys & buttons

```
0 - 9, n0 - n9,
f1 - f12,
a - z,
minus, mns, "-",
equal, eql, "=",
backspace, bspc,
left_bracket, lbrc, "[",
right_bracket, rbrc, "]",
backslash, bsls, r#"\"#,
semicolon, scln, ";",
quote, quot, "'",
comma, comm, ",",
dot, ".",
slash, sls, "/",
escape, esc,
grave, grv,
tab,
caps_lock, caps,
left_shift, lsft,
left_ctrl, lctrl,
left_win, lwin,
left_alt, lalt,
right_shift, rsft,
right_ctrl, rctrl,
right_win, rwin,
right_alt, ralt,
space, spc,
enter, ent,
print_screen, pscr,
scroll_lock, scrl,
pause, paus,
insert, ins,
delete, del,
home,
end,
page_up, pgup,
page_down, pgdn,
left,
up,
right,
down,
num_lock, num,
keypad0 - keypad9, kp0 - kp9,
keypad_slash, kp_sls, "kp/",
keypad_asterisk, kp_ast, "kp*",
keypad_minus, kp_mns, "kp-",
keypad_plus, kp_pls, "kp+",
keypad_dot, kp_dot, "kp.",

wheel_up,
wheel_down,

left_button, lb,
right_button, rb,
middle_button, mb,
back_button, xb1, bb,
forward_button, xb2, fb
```

# Macros

`click: { <wheel_up, wheel_down>: <key, button>, dur: <uint> }`: Simulate key/button presses by moving the scroll wheel. `dur` is the number of milliseconds to wait before sending a 'key release' event.

# Tasks

- `begin_pixel_cleaning`: Asus PG32UCDM only. Signal the monitor to begin pixel cleaning, executing any tasks enabled in `pixel_cleaning` beforehand. Tested on firmware MCM108 only.
- `go_to_sleep`: Suspend (sleep) the system.
- `let_walk_away`: Minimize all windows and enable the screensaver.
- `print_window_info`: Log properties of the foreground window and its relations, as well as all windows that are eligible to shift (see [Window shift](#window-shift)).
- `toggle_display_mode`: Enable/disable HDR mode and set color bit depth, dither state, and novideo_srgb state.
