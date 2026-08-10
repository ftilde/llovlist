mod storage;
mod sync;

use std::collections::BTreeSet;

use dioxus::prelude::*;
use kauf_core::{DeviceId, Edit, ItemName, Session, ShoppingList, Tag};
use serde::{Deserialize, Serialize};

const CSS: &str = include_str!("../assets/main.css");
const LIST_KEY: &str = "list.json";
const SETTINGS_KEY: &str = "settings.json";

fn main() {
    dioxus::launch(App);
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Settings {
    server: String,
    list: String,
    token: String,
    device: DeviceId,
}

impl Settings {
    fn load() -> Self {
        storage::load(SETTINGS_KEY).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or(
            Self {
                server: storage::default_server(),
                list: "home".to_owned(),
                token: String::new(),
                device: DeviceId::new(storage::random_u64()),
            },
        )
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string(self) {
            storage::save(SETTINGS_KEY, &json);
        }
    }
}

fn load_list() -> ShoppingList {
    storage::load(LIST_KEY).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

fn store(session: Signal<Session>) {
    if let Ok(json) = serde_json::to_string(session.read().list()) {
        storage::save(LIST_KEY, &json);
    }
}

fn apply(mut session: Signal<Session>, edit: Edit) {
    session.write().edit(edit);
    store(session);
}

fn undo(mut session: Signal<Session>) {
    session.write().undo();
    store(session);
}

fn add_item(session: Signal<Session>, mut draft: Signal<String>) {
    let name = ItemName::new(&draft.read()).ok();
    if let Some(name) = name {
        apply(session, Edit::add_item(name));
        draft.set(String::new());
    }
}

fn add_tag(
    on_edit: EventHandler<Edit>,
    name: ItemName,
    mut draft: Signal<String>,
    mut adding: Signal<bool>,
) {
    let tag = Tag::new(&draft.read()).ok();
    if let Some(tag) = tag {
        on_edit.call(Edit::AddTag { name, tag });
    }
    draft.set(String::new());
    adding.set(false);
}

fn start_sync(
    mut session: Signal<Session>,
    settings: Signal<Settings>,
    mut status: Signal<String>,
    mut error: Signal<String>,
) {
    let settings = settings.read().clone();
    let local = session.read().list().clone();
    spawn(async move {
        status.set("syncing…".to_owned());
        error.set(String::new());
        match sync::exchange(&settings.server, &settings.list, &settings.token, local).await {
            Ok(merged) => {
                session.write().merge(merged);
                store(session);
                let count = session.read().list().items().len();
                let plural = if count == 1 { "" } else { "s" };
                status.set(format!("synced, {count} item{plural}"));
            }
            Err(reason) => {
                status.set("sync failed".to_owned());
                error.set(reason);
            }
        }
    });
}

#[component]
fn App() -> Element {
    let settings = use_signal(Settings::load);
    let session = use_signal(|| Session::new(settings.read().device, load_list()));
    let status = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut draft = use_signal(String::new);
    let mut settings_open = use_signal(|| false);

    let items = session.read().list().items();
    let can_undo = session.read().can_undo();
    let message = status();
    let failure = error();

    rsx! {
        document::Style { {CSS} }
        header {
            h1 { "1kauf" }
            button { disabled: !can_undo, onclick: move |_| undo(session), "undo" }
            button { onclick: move |_| start_sync(session, settings, status, error), "sync" }
            button {
                class: if settings_open() { "active" },
                onclick: move |_| settings_open.toggle(),
                "settings"
            }
        }

        if settings_open() {
            SettingsPanel { settings }
        }

        div { class: "add",
            input {
                placeholder: "add an item",
                value: "{draft}",
                oninput: move |event| draft.set(event.value()),
                onkeydown: move |event: KeyboardEvent| {
                    if event.key() == Key::Enter {
                        add_item(session, draft);
                    }
                },
            }
            button { onclick: move |_| add_item(session, draft), "+" }
        }

        if items.is_empty() {
            p { class: "empty", "nothing to buy" }
        }

        ul { class: "items",
            for (name, tags) in items {
                ItemRow { key: "{name}", name, tags, on_edit: move |edit| apply(session, edit) }
            }
        }

        if !message.is_empty() {
            p { class: "status", "{message}" }
        }

        if !failure.is_empty() {
            div { class: "overlay",
                div { class: "card",
                    h2 { "sync failed" }
                    p { "{failure}" }
                    button { onclick: move |_| error.set(String::new()), "ok" }
                }
            }
        }
    }
}

#[component]
fn ItemRow(name: ItemName, tags: BTreeSet<Tag>, on_edit: EventHandler<Edit>) -> Element {
    let mut adding = use_signal(|| false);
    let mut draft = use_signal(String::new);

    rsx! {
        li { class: "item",
            button {
                class: "tick",
                title: "got it",
                onclick: {
                    let name = name.clone();
                    move |_| on_edit.call(Edit::RemoveItem { name: name.clone() })
                },
                "✓"
            }
            span { class: "name", "{name}" }
            div { class: "tags",
                for tag in tags {
                    button {
                        key: "{tag}",
                        class: "tag",
                        onclick: {
                            let (name, tag) = (name.clone(), tag.clone());
                            move |_| {
                                on_edit.call(Edit::RemoveTag { name: name.clone(), tag: tag.clone() })
                            }
                        },
                        "{tag} ×"
                    }
                }
                if adding() {
                    input {
                        class: "tag-input",
                        placeholder: "2x",
                        autofocus: true,
                        value: "{draft}",
                        oninput: move |event| draft.set(event.value()),
                        onkeydown: {
                            let name = name.clone();
                            move |event: KeyboardEvent| match event.key() {
                                Key::Enter => add_tag(on_edit, name.clone(), draft, adding),
                                Key::Escape => adding.set(false),
                                _ => {}
                            }
                        },
                    }
                } else {
                    button { class: "add-tag", onclick: move |_| adding.set(true), "+" }
                }
            }
        }
    }
}

#[component]
fn SettingsPanel(settings: Signal<Settings>) -> Element {
    let current = settings.read().clone();
    let device = format!("{:?}", current.device);

    let mut update = move |change: fn(&mut Settings, String), value: String| {
        change(&mut settings.write(), value);
        settings.read().save();
    };

    rsx! {
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
                "list"
                input {
                    value: "{current.list}",
                    oninput: move |event| update(|s, v| s.list = v, event.value()),
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
