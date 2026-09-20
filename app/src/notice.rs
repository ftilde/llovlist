//! A short message at the bottom of the screen that goes away on its own.

use dioxus::prelude::*;

#[derive(Clone, PartialEq, Default)]
pub struct Notice {
    text: String,
    // Distinguishes a newer notice from the one whose timer is running out.
    serial: u64,
}

pub fn show(mut notice: Signal<Notice>, text: String) {
    let serial = notice.read().serial + 1;
    notice.set(Notice { text, serial });
    spawn(async move {
        // The webview's timer; no async runtime dependency needed for a delay.
        let _ = document::eval("return new Promise((done) => setTimeout(done, 3000));").await;
        if notice.read().serial == serial {
            notice.set(Notice::default());
        }
    });
}

#[component]
pub fn Notices() -> Element {
    let notice: Signal<Notice> = use_context();
    let message = notice.read().text.clone();
    rsx! {
        if !message.is_empty() {
            p { class: "notice", "{message}" }
        }
    }
}
