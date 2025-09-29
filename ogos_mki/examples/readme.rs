#![allow(clippy::uninlined_format_args)]

use mki::{bind_key, Action, InhibitEvent, Key, Sequence};
use std::thread;
use std::time::Duration;

fn main() {
    Key::A.bind(|_| {
        println!("A pressed, sending B");
        Key::B.click();
    });
    mki::bind_any_key(Action::handle_kb(|key| {
        use Key::*;
        if matches!(key, S | L | O | W | LeftShift | LeftCtrl | B) {
            // Ignore outputs from other commands for nicer output
        } else {
            println!("Some key pressed pressed: {:?}", key);
        }
    }));
    mki::bind_any_button(Action::handle_mouse(|button| {
        println!("Mouse button pressed {:?}", button);
    }));
    mki::register_hotkey(&[Key::LeftCtrl, Key::B], || {
        println!("Ctrl+B Pressed")
    });
    mki::bind_key(
        Key::S,
        Action::sequencing_kb(|_| {
            Sequence::text("LLLLLow").unwrap().send();
            thread::sleep(Duration::from_secs(1));
        }),
    );

    // This binds action to a W key,
    // that W press will not be sent to the following services ( only on windows )
    // whenever Caps Lock is toggled
    // Action will be executed on separate thread.
    bind_key(
        Key::W,
        Action {
            callback: Box::new(|event, state| {
                println!("key: {:?} changed state now is: {:?}", event, state);
            }),
            inhibit: InhibitEvent::maybe(|| {
                if Key::CapsLock.is_toggled() {
                    InhibitEvent::Yes
                } else {
                    InhibitEvent::No
                }
            }),
            sequencer: false,
            defer: true,
        },
    );

    thread::sleep(Duration::from_secs(100));
}
