use crate::{
    common::*,
    config::{self, *},
    display::*,
    err::*
};
#[cfg(feature = "dbg_window_info")]
use crate::win32::*;

use cfg_if::*;
use log::*;
use mki::{
    InputEvent::*,
    Key,
    Button,
    *
};
use qmk_via_api::{
    api as qmk_api,
    keycodes::*
};
use serde::*;
use std::{
    cell::*,
    collections::*,
    fs,
    str::*,
    sync::mpsc::{self, *},
    thread,
    time::*
};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::Power::*
    }
};

#[cfg(feature = "dbg_window_info")]
use windows::Win32::UI::WindowsAndMessaging::*;

pub(crate) mod qmk_deser {
    use super::*;

    #[derive(Deserialize)]
    pub(crate) struct KeyCoord {
        pub(crate) col: u8,
        pub(crate) row: u8,
        pub(crate) val: u16
    }

    #[derive(Deserialize)]
    #[serde(transparent)]
    pub(crate) struct Layer {
        pub(crate) key_coords: Vec<KeyCoord>
    }

    #[derive(Deserialize)]
    pub(crate) struct Layout {
        #[serde(rename = "keymap")]
        pub(crate) layers: Vec<Layer>
    }
}

mod trigger_watch {
    use super::*;

    pub(crate) unsafe fn begin(hotkey_tasks: HashMap<Key, Task>, pixel_cleaning_prelude: Option<PixelCleaning>, rx: Receiver<InputEvent>) {
        for trigger in rx.iter() {
            if let InputEvent::Keyboard(key) = trigger && let Some(task) = hotkey_tasks.get(&key) {
                match task {
                    Task::BeginPixelCleaning => begin_pixel_cleaning(pixel_cleaning_prelude).unwrap_or_else(|err| {
                        error!("{}: failure during pixel cleaning: {}", module_path!(), err);
                    }),
                    Task::LetWalkAway => let_walk_away().unwrap_or_else(|err| {
                        error!("{}: failed to let walk away: {}", module_path!(), err);
                    }),
                    Task::SetSleepMode => _ = SetSuspendState(false, false, true).win32_core_ok().x().inspect_err(|err| {
                        error!("{}: failed to set sleep mode: {}", module_path!(), err);
                    }),
                    Task::ToggleDisplayMode => _ = set_display_mode(SetDisplayModeOp::Toggle).inspect_err(|err| {
                        error!("{}: failed to toggle display mode: {}", module_path!(), err);
                    }),
                    #[cfg(feature = "dbg_window_info")]
                    Task::GetForegroundInfo => {
                        print_foreground_info().unwrap_or_else(|err| {
                            error!("{}: failed to get foreground info: {}", module_path!(), err);
                        });

                        print_eligible_for_shift_info().unwrap_or_else(|err| {
                            error!("{}: failed to get eligible-for-shift info: {}", module_path!(), err);
                        });
                    }
                }
            }
        }

        info!("{}: closed", module_path!());
    }
}

struct EligibleForShiftInfo {
    eligibles: Vec<HWND>,
    screen_extent: Extent2d
}

struct KeyCoord {
    row: u8,
    col: u8
}

pub(crate) struct Qmk {
    api: qmk_api::KeyboardApi,
    layer: u8,
    layout: HashMap<Key, KeyCoord>
}
impl Qmk {
    pub(crate) fn new(vid: u16, pid: u16, usage_page: u16, qmk_config: &config::Qmk) -> Res<Self> {
        let api = qmk_api::KeyboardApi::new(vid, pid, usage_page)
            .map_err(|_| ErrVar::FailedQmkKeyboardInit { vid, pid, usage_page })?;

        let layout_str = fs::read_to_string(qmk_config.layout_path.as_str())?;
        let layout_deser = serde_json::from_str::<qmk_deser::Layout>(&layout_str)?;

        let mut layout = HashMap::new();
        for key_coord in &layout_deser.layers.get(qmk_config.layer as usize).ok_or(ErrVar::InvalidQmkLayer { index: qmk_config.layer })?.key_coords {
            let key = match Keycode::try_from(key_coord.val).ok()
                .and_then(|keycode| keycode.try_as_key().ok())
            {
                Some(key) => key,
                None => continue
            };

            layout.insert(key, KeyCoord { row: key_coord.row, col: key_coord.col });
        }

        Ok(Self {
            api,
            layer: qmk_config.layer,
            layout
        })
    }
}

#[derive(Default)]
struct ThreadState {
    trigger_to_send: Option<InputEvent>,
    trigger_is_pressed: bool,
    prefixes_pressed: HashSet<Key>
}

struct TopLevelSiblingsInfo {
    fg_pid: u32,
    siblings: Vec<HWND>
}

thread_local! {
    static THREAD_STATE: RefCell<ThreadState> = default!();
}

pub(crate) trait AsKeycode {
    fn as_keycode(&self) -> Keycode;
}
impl AsKeycode for Key {
    fn as_keycode(&self) -> Keycode {
        use Key::*;
        use Keycode::*;

        match self {
            Escape => KC_ESCAPE,
            F1 => KC_F1,
            F2 => KC_F2,
            F3 => KC_F3,
            F4 => KC_F4,
            F5 => KC_F5,
            F6 => KC_F6,
            F7 => KC_F7,
            F8 => KC_F8,
            F9 => KC_F9,
            F10 => KC_F10,
            F11 => KC_F11,
            F12 => KC_F12,
            PrintScreen => KC_PRINT_SCREEN,
            ScrollLock => KC_SCROLL_LOCK,
            Pause => KC_PAUSE,
            Grave => KC_GRAVE,
            N0  => KC_0,
            N1  => KC_1,
            N2  => KC_2,
            N3  => KC_3,
            N4  => KC_4,
            N5  => KC_5,
            N6  => KC_6,
            N7  => KC_7,
            N8  => KC_8,
            N9  => KC_9,
            Minus => KC_MINUS,
            Equal => KC_EQUAL,
            A => KC_A,
            B => KC_B,
            C => KC_C,
            D => KC_D,
            E => KC_E,
            F => KC_F,
            G => KC_G,
            H => KC_H,
            I => KC_I,
            J => KC_J,
            K => KC_K,
            L => KC_L,
            M => KC_M,
            N => KC_N,
            O => KC_O,
            P => KC_P,
            Q => KC_Q,
            R => KC_R,
            S => KC_S,
            T => KC_T,
            U => KC_U,
            V => KC_V,
            W => KC_W,
            X => KC_X,
            Y => KC_Y,
            Z => KC_Z,
            LeftBracket => KC_LEFT_BRACKET,
            RightBracket => KC_RIGHT_BRACKET,
            Backslash => KC_BACKSLASH,
            Semicolon => KC_SEMICOLON,
            Quote => KC_QUOTE,
            Comma => KC_COMMA,
            Dot => KC_DOT,
            Slash => KC_SLASH,
            Tab => KC_TAB,
            CapsLock => KC_CAPS_LOCK,
            LeftShift => KC_LEFT_SHIFT,
            LeftCtrl => KC_LEFT_CTRL,
            LeftWin => KC_LEFT_GUI,
            LeftAlt => KC_LEFT_ALT,
            Space => KC_SPACE,
            Backspace => KC_BACKSPACE,
            Enter => KC_ENTER,
            RightShift => KC_RIGHT_SHIFT,
            RightCtrl => KC_RIGHT_CTRL,
            RightWin => KC_RIGHT_GUI,
            RightAlt => KC_RIGHT_ALT,
            Insert => KC_INSERT,
            Delete => KC_DELETE,
            Home => KC_HOME,
            End => KC_END,
            PageUp => KC_PAGE_UP,
            PageDown => KC_PAGE_DOWN,
            Left => KC_LEFT,
            Up => KC_UP,
            Right => KC_RIGHT,
            Down => KC_DOWN,
            NumLock => KC_NUM_LOCK,
            Keypad0 => KC_KP_0,
            Keypad1 => KC_KP_1,
            Keypad2 => KC_KP_2,
            Keypad3 => KC_KP_3,
            Keypad4 => KC_KP_4,
            Keypad5 => KC_KP_5,
            Keypad6 => KC_KP_6,
            Keypad7 => KC_KP_7,
            Keypad8 => KC_KP_8,
            Keypad9 => KC_KP_9,
            KeypadSlash => KC_KP_SLASH,
            KeypadAsterisk => KC_KP_ASTERISK,
            KeypadMinus => KC_KP_MINUS,
            KeypadPlus => KC_KP_PLUS,
            KeypadDot => KC_KP_DOT
        }
    }
}

pub(crate) trait TryAsInputEvent {
    fn try_as_input_event(&self) -> ResVar<InputEvent>;
}
impl TryAsInputEvent for &str {
    fn try_as_input_event(&self) -> ResVar<InputEvent> {
        Key::from_str(self).map(Keyboard)
            .or_else(|_| {
                Button::from_str(self).map(MouseButton)
            })
            .or_else(|_| {
                Wheel::from_str(self).map(MouseWheel)
            })
            .map_err(|_| ErrVar::FailedInputEventFrom { from: (*self).into() })
    }
}

pub(crate) trait TryAsKey {
    fn try_as_key(&self) -> ResVar<Key>;
}
impl TryAsKey for &str {
    fn try_as_key(&self) -> ResVar<Key> {
        Key::from_str(self).map_err(|_| ErrVar::FailedKeyFromStr { from: (*self).into() })
    }
}
impl TryAsKey for Keycode {
    fn try_as_key(&self) -> ResVar<Key> {
        use Key::*;
        use Keycode::*;

        Ok(match *self {
            KC_ESCAPE => Escape,
            KC_F1 => F1,
            KC_F2 => F2,
            KC_F3 => F3,
            KC_F4 => F4,
            KC_F5 => F5,
            KC_F6 => F6,
            KC_F7 => F7,
            KC_F8 => F8,
            KC_F9 => F9,
            KC_F10 => F10,
            KC_F11 => F11,
            KC_F12 => F12,
            KC_PRINT_SCREEN => PrintScreen,
            KC_SCROLL_LOCK => ScrollLock,
            KC_PAUSE => Pause,
            KC_GRAVE => Grave,
            KC_0 => N0,
            KC_1 => N1,
            KC_2 => N2,
            KC_3 => N3,
            KC_4 => N4,
            KC_5 => N5,
            KC_6 => N6,
            KC_7 => N7,
            KC_8 => N8,
            KC_9 => N9,
            KC_MINUS => Minus,
            KC_EQUAL => Equal,
            KC_A => A,
            KC_B => B,
            KC_C => C,
            KC_D => D,
            KC_E => E,
            KC_F => F,
            KC_G => G,
            KC_H => H,
            KC_I => I,
            KC_J => J,
            KC_K => K,
            KC_L => L,
            KC_M => M,
            KC_N => N,
            KC_O => O,
            KC_P => P,
            KC_Q => Q,
            KC_R => R,
            KC_S => S,
            KC_T => T,
            KC_U => U,
            KC_V => V,
            KC_W => W,
            KC_X => X,
            KC_Y => Y,
            KC_Z => Z,
            KC_LEFT_BRACKET => LeftBracket,
            KC_RIGHT_BRACKET => RightBracket,
            KC_BACKSLASH => Backslash,
            KC_SEMICOLON => Semicolon,
            KC_QUOTE => Quote,
            KC_COMMA => Comma,
            KC_DOT => Dot,
            KC_SLASH => Slash,
            KC_TAB => Tab,
            KC_CAPS_LOCK => CapsLock,
            KC_LEFT_SHIFT => LeftShift,
            KC_LEFT_CTRL => LeftCtrl,
            KC_LEFT_GUI => LeftWin,
            KC_LEFT_ALT => LeftAlt,
            KC_SPACE => Space,
            KC_BACKSPACE => Backspace,
            KC_ENTER => Enter,
            KC_RIGHT_SHIFT => RightShift,
            KC_RIGHT_CTRL => RightCtrl,
            KC_RIGHT_GUI => RightWin,
            KC_RIGHT_ALT => RightAlt,
            KC_INSERT => Insert,
            KC_DELETE => Delete,
            KC_HOME => Home,
            KC_END => End,
            KC_PAGE_UP => PageUp,
            KC_PAGE_DOWN => PageDown,
            KC_LEFT => Left,
            KC_UP => Up,
            KC_RIGHT => Right,
            KC_DOWN => Down,
            KC_NUM_LOCK => NumLock,
            KC_KP_0 => Keypad0,
            KC_KP_1 => Keypad1,
            KC_KP_2 => Keypad2,
            KC_KP_3 => Keypad3,
            KC_KP_4 => Keypad4,
            KC_KP_5 => Keypad5,
            KC_KP_6 => Keypad6,
            KC_KP_7 => Keypad7,
            KC_KP_8 => Keypad8,
            KC_KP_9 => Keypad9,
            KC_KP_SLASH => KeypadSlash,
            KC_KP_ASTERISK => KeypadAsterisk,
            KC_KP_MINUS => KeypadMinus,
            KC_KP_PLUS => KeypadPlus,
            KC_KP_DOT => KeypadDot,
            _ => Err(ErrVar::FailedKeyFromKeycode { from: self.clone() })?
        })
    }
}

cfg_if! { if #[cfg(feature = "dbg_window_info")] {
    unsafe extern "system" fn enum_windows_eligible_for_shift_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let EligibleForShiftInfo { eligibles, screen_extent } = &mut *(lparam.0 as *mut _);

        if hwnd.is_eligible_for_shift(*screen_extent).unwrap_or_default() {
            eligibles.push(hwnd);
        }

        TRUE
    }

    unsafe extern "system" fn enum_child_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let children = &mut *(lparam.0 as *mut Vec<HWND>);
        children.push(hwnd);

        TRUE
    }

    unsafe extern "system" fn enum_windows_tl_siblings_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let TopLevelSiblingsInfo { fg_pid, siblings } = &mut *(lparam.0 as *mut _);

        let mut win_pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut win_pid));

        if win_pid == *fg_pid && IsWindowVisible(hwnd).as_bool() {
            siblings.push(hwnd);
        }

        TRUE
    }

    unsafe fn _is_top_level_window(hwnd: HWND) -> bool {
        let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
        let parent = hwnd.get_parent().unwrap_or_default();

        let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
        let is_child_window = style & WS_CHILD == WS_CHILD;
        let is_owned_popup = !owner.is_invalid() && (style & WS_POPUP == WS_POPUP);

        !is_child_window && (parent.is_invalid() || is_owned_popup)
    }

    unsafe fn print_eligible_for_shift_info() -> Res<()> {
        let screen_extent = get_screen_extent()?;
        let mut eligible_for_shift_info = EligibleForShiftInfo {
            eligibles: Vec::new(),
            screen_extent
        };
        EnumWindows(Some(enum_windows_eligible_for_shift_proc), LPARAM(&mut eligible_for_shift_info as *mut _ as _))?;

        info!("{}: eligible for shift:", module_path!());
        for hwnd in eligible_for_shift_info.eligibles {
            let exe = hwnd.get_exe_or_err();
            let tpids = hwnd.get_thread_proc_ids().unwrap_or_default();
            let caption = hwnd.get_caption_or_err();
            let class = hwnd._get_class_or_err();
            let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
            let parent = hwnd.get_parent().unwrap_or_default();

            info!("{}: \thwnd: {:p}, exe: {}, tid: {}, caption: {}, class: {}, owner: {:p}, parent: {:p}",
                module_path!(), hwnd.0, exe, tpids.thread, caption, class, owner.0, parent.0);
        }

        Ok(())
    }

    unsafe fn print_foreground_info() -> Res<()> {
        let fg_hwnd = GetForegroundWindow();
        let fg_exe = fg_hwnd.get_exe_or_err();
        let fg_tpids = fg_hwnd.get_thread_proc_ids().unwrap_or_default();
        let fg_caption = fg_hwnd.get_caption_or_err();
        let fg_class = fg_hwnd._get_class_or_err();
        let fg_owner = GetWindow(fg_hwnd, GW_OWNER).unwrap_or_default();
        let fg_parent = fg_hwnd.get_parent().unwrap_or_default();

        info!("{}: fg:", module_path!());
        info!("{}: \thwnd: {:p}, exe: {}, tid: {}, caption: {}, class: {}, owner: {:p}, parent: {:p}",
            module_path!(), fg_hwnd.0, fg_exe, fg_tpids.thread, fg_caption, fg_class, fg_owner.0, fg_parent.0);

        let mut children: Vec<HWND> = Vec::new();
        _ = EnumChildWindows(Some(fg_hwnd), Some(enum_child_windows_proc), LPARAM(&mut children as *mut _ as _));

        let mut tl_siblings_info = TopLevelSiblingsInfo {
            fg_pid: fg_tpids.proc,
            siblings: Vec::new()
        };
        EnumWindows(Some(enum_windows_tl_siblings_proc), LPARAM(&mut tl_siblings_info as *mut _ as _))?;

        info!("{}: fg children:", module_path!());
        for hwnd in children {
            let tpids = hwnd.get_thread_proc_ids().unwrap_or_default();
            let caption = hwnd.get_caption_or_err();
            let class = hwnd._get_class_or_err();
            let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
            let parent = hwnd.get_parent().unwrap_or_default();
            info!("{}: \thwnd: {:p}, tid: {}, caption: {}, class: {}, owner: {:p}, parent: {:p}",
                module_path!(), hwnd.0, tpids.thread, caption, class, owner.0, parent.0);
        }

        let fg_root_owner = GetAncestor(fg_hwnd, GA_ROOTOWNER);
        info!("{}: fg top level siblings:", module_path!());
        for hwnd in tl_siblings_info.siblings {
            if hwnd == fg_root_owner { continue }

            let tpids = hwnd.get_thread_proc_ids().unwrap_or_default();
            let caption = hwnd.get_caption_or_err();
            let class = hwnd._get_class_or_err();
            let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
            let parent = hwnd.get_parent().unwrap_or_default();
            info!("{}: \thwnd: {:p}, tid: {}, caption: {}, class: {}, owner: {:p}, parent: {:p}",
                module_path!(), hwnd.0, tpids.thread, caption, class, owner.0, parent.0);
        }

        Ok(())
    }
} }

pub(crate) fn click_with_sleep(event: InputEvent) {
    match event {
        Keyboard(key) => {
            key.press();
            thread::sleep(Duration::from_millis(30));
            key.release();
        },
        MouseButton(button) => {
            button.press();
            thread::sleep(Duration::from_millis(30));
            button.release();
        },
        _ => ()
    }
}

pub(crate) fn unmap_mki(from: InputEvent) {
    match from {
        InputEvent::Keyboard(key) => mki::remove_key_bind(key),
        InputEvent::MouseButton(button) => mki::remove_button_bind(button),
        InputEvent::MouseWheel(wheel) => mki::remove_wheel_bind(wheel)
    }
}

pub(crate) fn map_qmk(qmk: &Qmk, from: Key, to: Keycode) {
    if let Some(coord) = qmk.layout.get(&from) {
        _ = qmk.api.set_key(qmk.layer, coord.row, coord.col, to.clone() as u16); // Ignore response from KeyboardApi::hid_command(ViaCommandId::DynamicKeymapSetKeycode, ..)
    }
}

pub(crate) fn unmap_qmk(qmk: &Qmk, from: Key) {
    if let Some(coord) = qmk.layout.get(&from) {
        _ = qmk.api.set_key(qmk.layer, coord.row, coord.col, from.as_keycode() as u16);
    }
}

pub(crate) fn set_bind(binds_config: &Binds, msg: BindMsg)  {
    match msg {
        BindMsg::Bind(BindName::Underscore) => {
            if let Some(underscore) = binds_config.underscore {
                let action = Action {
                    callback: Box::new(move |_, state| {
                        if state == State::Pressed && underscore.while_pressed.is_pressed() {
                            click_with_sleep(Keyboard(Key::Minus));
                        }
                    }),
                    inhibit: InhibitEvent::maybe(move || {
                        match underscore.while_pressed.is_pressed() {
                            true => InhibitEvent::Yes,
                            false => InhibitEvent::No
                        }
                    }),
                    defer: true,
                    sequencer: false
                };

                underscore.act_on.act_on(action);
            }
        },
        BindMsg::Unbind(BindName::Underscore) => {
            if let Some(underscore) = binds_config.underscore {
                mki::remove_key_bind(underscore.act_on);
            }
        }
    }
}

pub(crate) unsafe fn configure_static_binds() -> Res<()> {
    let config = config::get().read()?;
    let binds_config = config.binds.as_ref().ok_or(ErrVar::MissingConfigKey { name: Binds::NAME })?;

    if let Some(hotkeys) = binds_config.hotkeys.as_ref() {
        #[allow(unused_mut)]
        let mut hotkey_tasks = hotkeys.maps.clone();
        #[cfg(feature = "dbg_window_info")]
        hotkey_tasks.insert(Key::F, Task::GetForegroundInfo);

        // Invoke tasks
        let pixel_cleaning_prelude = config.pixel_cleaning;
        let (invoke_task_sx, rx) = mpsc::channel::<InputEvent>();
        thread::spawn(move || trigger_watch::begin(hotkey_tasks, pixel_cleaning_prelude, rx));

        for prefix in hotkeys.prefix.iter() {
            let invoke_task_sx = invoke_task_sx.clone();

            let callback = Box::new(move |event, state| {
                match state {
                    State::Pressed => if let InputEvent::Keyboard(key) = event {
                        THREAD_STATE.with_borrow_mut(|ts| {
                            ts.prefixes_pressed.insert(key);
                        });
                    },
                    State::Released => THREAD_STATE.with_borrow_mut(|ts| {
                        if let InputEvent::Keyboard(key) = event {
                            ts.prefixes_pressed.remove(&key);
                        }

                        if ts.prefixes_pressed.is_empty() && !ts.trigger_is_pressed && let Some(trigger) = ts.trigger_to_send.take() {
                            invoke_task_sx.send(trigger).unwrap();
                        }
                    }),
                    _ => ()
                }
            });
            let action = Action {
                callback,
                inhibit: InhibitEvent::No,
                // Don't bother offloading - the callback is processed quickly enough on the LL hook thread
                defer: false,
                sequencer: false
            };

            prefix.act_on(action);
        }

        let hotkeys_prefix_len = hotkeys.prefix.len();
        let hotkeys_triggers_iter = hotkeys.maps.keys();
        #[cfg(feature = "dbg_window_info")]
        let hotkeys_triggers_iter = hotkeys_triggers_iter.chain([&Key::F]);

        for trigger in hotkeys_triggers_iter {
            let invoke_task_sx = invoke_task_sx.clone();

            let callback = Box::new(move |event, state| {
                match state {
                    State::Pressed => THREAD_STATE.with_borrow_mut(|ts| {
                        ts.trigger_is_pressed = true;

                        if ts.prefixes_pressed.len() == hotkeys_prefix_len && ts.trigger_to_send.is_none() {
                            ts.trigger_to_send = Some(event);
                        }
                    }),
                    State::Released => THREAD_STATE.with_borrow_mut(|ts| {
                        ts.trigger_is_pressed = false;

                        if ts.prefixes_pressed.is_empty() && let Some(trigger) = ts.trigger_to_send.take() {
                            invoke_task_sx.send(trigger).unwrap();
                        }
                    }),
                    _ => ()
                }
            });
            let action = Action {
                callback,
                inhibit: InhibitEvent::No,
                defer: false,
                sequencer: false
            };

            trigger.act_on(action);
        }
    }

    if let Some(underscore) = binds_config.underscore {
        underscore.act_on.act_on(
            Action {
                callback: Box::new(move |_, state| {
                    if state == State::Pressed && underscore.while_pressed.is_pressed() {
                        click_with_sleep(Keyboard(Key::Minus));
                    }
                }),
                inhibit: InhibitEvent::maybe(move || {
                    match underscore.while_pressed.is_pressed() {
                        true => InhibitEvent::Yes,
                        false => InhibitEvent::No
                    }
                }),
                defer: false,
                sequencer: true
            }
        );
    }

    info!("{}: configured", module_path!());

    Ok(())
}
