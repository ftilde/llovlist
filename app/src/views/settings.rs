use dioxus::prelude::*;

use crate::state::Settings;

#[component]
pub fn SettingsView() -> Element {
    let mut settings: Signal<Settings> = use_context();
    let current = settings.read().clone();
    let device = format!("{:?}", current.device);

    let mut update = move |change: fn(&mut Settings, String), value: String| {
        change(&mut settings.write(), value);
        settings.read().save();
    };

    rsx! {
        main { class: "page",
            section { class: "settings",
                label {
                    "server"
                    input {
                        placeholder: "https://host:8080",
                        value: "{current.server}",
                        oninput: move |event| update(|s, v| s.server = v, event.value()),
                    }
                }
                label {
                    "token"
                    input {
                        r#type: "password",
                        value: "{current.token}",
                        oninput: move |event| update(|s, v| s.token = v, event.value()),
                    }
                }
                p { class: "device", "this device: {device}" }
            }
        }
    }
}
