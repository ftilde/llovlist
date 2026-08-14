mod storage;
mod sync;

use std::collections::BTreeSet;

use dioxus::prelude::*;
use kauf_core::{DeviceId, Edit, ItemName, ListName, Lists, Session, Tag};
use serde::{Deserialize, Serialize};

const CSS: &str = include_str!("../assets/main.css");
const LISTS_KEY: &str = "lists.json";
const SETTINGS_KEY: &str = "settings.json";

// dioxus re-exports the same webview crate under two names, one per feature.
#[cfg(all(feature = "desktop", not(feature = "mobile")))]
use dioxus::desktop as webview;
#[cfg(feature = "mobile")]
use dioxus::mobile as webview;

fn main() {
    #[cfg(feature = "desktop")]
    {
        // GTK ignores xdg-decoration, so a tiling WM cannot remove the title bar itself.
        let window = webview::WindowBuilder::new()
            .with_decorations(false)
            .with_title("1kauf");
        dioxus::LaunchBuilder::desktop()
            .with_cfg(webview_config().with_window(window))
            .launch(App);
    }
    #[cfg(all(feature = "mobile", not(feature = "desktop")))]
    dioxus::LaunchBuilder::mobile()
        .with_cfg(webview_config())
        .launch(App);
    #[cfg(not(any(feature = "desktop", feature = "mobile")))]
    dioxus::launch(App);
}

#[cfg(any(feature = "desktop", feature = "mobile"))]
fn webview_config() -> webview::Config {
    // Release builds otherwise preventDefault every contextmenu event, taking the
    // long-press paste menu with it.
    webview::Config::new().with_disable_context_menu(false)
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Settings {
    server: String,
    token: String,
    device: DeviceId,
}

impl Settings {
    fn load() -> Self {
        storage::load(SETTINGS_KEY)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or(Self {
                server: String::new(),
                token: String::new(),
                device: DeviceId::new(storage::random_u64()),
            })
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string(self) {
            storage::save(SETTINGS_KEY, &json);
        }
    }
}

fn load_lists() -> Lists {
    storage::load(LISTS_KEY)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn store(session: Signal<Session>) {
    if let Ok(json) = serde_json::to_string(session.read().lists()) {
        storage::save(LISTS_KEY, &json);
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

fn add_item(session: Signal<Session>, list: ListName, mut draft: Signal<String>) {
    let name = ItemName::new(&draft.read()).ok();
    if let Some(name) = name {
        let key = session.read().item_key_at_end(&list);
        apply(session, Edit::add_item(list, name, key));
        draft.set(String::new());
    }
}

fn move_item(session: Signal<Session>, list: &ListName, name: &ItemName, offset: isize) {
    let key = session.read().item_key_for_move(list, name, offset);
    if let Some(key) = key {
        apply(
            session,
            Edit::MoveItem {
                list: list.clone(),
                name: name.clone(),
                key,
            },
        );
    }
}

fn create_list(
    session: Signal<Session>,
    mut current: Signal<Option<ListName>>,
    mut draft: Signal<String>,
) {
    let list = ListName::new(&draft.read()).ok();
    if let Some(list) = list {
        let key = session.read().key_at_end();
        apply(
            session,
            Edit::CreateList {
                list: list.clone(),
                key,
            },
        );
        current.set(Some(list));
        draft.set(String::new());
    }
}

fn move_list(session: Signal<Session>, list: &ListName, offset: isize) {
    let key = session.read().key_for_move(list, offset);
    if let Some(key) = key {
        apply(
            session,
            Edit::MoveList {
                list: list.clone(),
                key,
            },
        );
    }
}

fn add_tag(
    on_edit: EventHandler<Edit>,
    list: ListName,
    name: ItemName,
    mut draft: Signal<String>,
    mut adding: Signal<bool>,
) {
    let tag = Tag::new(&draft.read()).ok();
    if let Some(tag) = tag {
        on_edit.call(Edit::AddTag { list, name, tag });
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
    let local = session.read().lists().clone();
    spawn(async move {
        status.set("syncing…".to_owned());
        error.set(String::new());
        match sync::exchange(&settings.server, &settings.token, local).await {
            Ok(merged) => {
                session.write().merge(merged);
                store(session);
                let count = session.read().lists().names().len();
                let plural = if count == 1 { "" } else { "s" };
                status.set(format!("synced, {count} list{plural}"));
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
    let session = use_signal(|| Session::new(settings.read().device, load_lists()));
    let status = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut current = use_signal(|| session.read().lists().names().first().cloned());
    let mut draft = use_signal(String::new);
    let mut new_list = use_signal(String::new);
    let mut settings_open = use_signal(|| false);

    let names = session.read().lists().names();
    let can_undo = session.read().can_undo();
    let message = status();
    let failure = error();

    // Falling back to the first name keeps a list selected when the current one is
    // deleted here or disappears in a merge.
    let selected = current()
        .filter(|list| names.contains(list))
        .or_else(|| names.first().cloned());
    let items = selected
        .as_ref()
        .map(|list| session.read().lists().items(list))
        .unwrap_or_default();

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

        div { class: "body",
            nav { class: "tabs",
                for name in names.clone() {
                    button {
                        key: "{name}",
                        class: if Some(&name) == selected.as_ref() { "tab active" } else { "tab" },
                        onclick: {
                            let name = name.clone();
                            move |_| current.set(Some(name.clone()))
                        },
                        "{name}"
                    }
                }
                input {
                    class: "new-list",
                    placeholder: "+ list",
                    value: "{new_list}",
                    oninput: move |event| new_list.set(event.value()),
                    onkeydown: move |event: KeyboardEvent| {
                        if event.key() == Key::Enter {
                            create_list(session, current, new_list);
                        }
                    },
                }
                if let Some(list) = selected.clone() {
                    div { class: "reorder",
                        button {
                            title: "move up",
                            onclick: {
                                let list = list.clone();
                                move |_| move_list(session, &list, -1)
                            },
                            "↑"
                        }
                        button {
                            title: "move down",
                            onclick: {
                                let list = list.clone();
                                move |_| move_list(session, &list, 1)
                            },
                            "↓"
                        }
                    }
                }
            }

            main {
                if let Some(list) = selected.clone() {
                    div { class: "add",
                        input {
                            placeholder: "add an item",
                            value: "{draft}",
                            oninput: move |event| draft.set(event.value()),
                            onkeydown: {
                                let list = list.clone();
                                move |event: KeyboardEvent| {
                                    if event.key() == Key::Enter {
                                        add_item(session, list.clone(), draft);
                                    }
                                }
                            },
                        }
                        button {
                            onclick: {
                                let list = list.clone();
                                move |_| add_item(session, list.clone(), draft)
                            },
                            "+"
                        }
                    }

                    if items.is_empty() {
                        p { class: "empty", "nothing to buy" }
                    }

                    ul { class: "items",
                        for (name, tags) in items.clone() {
                            ItemRow {
                                key: "{name}",
                                list: list.clone(),
                                name,
                                tags,
                                session,
                                on_edit: move |edit| apply(session, edit),
                            }
                        }
                    }

                    button {
                        class: "delete-list",
                        onclick: {
                            let list = list.clone();
                            move |_| apply(session, Edit::RemoveList { list: list.clone() })
                        },
                        "delete list “{list}”"
                    }
                } else {
                    p { class: "empty", "no list yet — name one on the left" }
                }
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
fn ItemRow(
    list: ListName,
    name: ItemName,
    tags: BTreeSet<Tag>,
    session: Signal<Session>,
    on_edit: EventHandler<Edit>,
) -> Element {
    let mut adding = use_signal(|| false);
    let mut draft = use_signal(String::new);

    rsx! {
        li { class: "item",
            div { class: "shift",
                button {
                    title: "move up",
                    onclick: {
                        let (list, name) = (list.clone(), name.clone());
                        move |_| move_item(session, &list, &name, -1)
                    },
                    "↑"
                }
                button {
                    title: "move down",
                    onclick: {
                        let (list, name) = (list.clone(), name.clone());
                        move |_| move_item(session, &list, &name, 1)
                    },
                    "↓"
                }
            }
            button {
                class: "tick",
                title: "got it",
                onclick: {
                    let (list, name) = (list.clone(), name.clone());
                    move |_| {
                        on_edit
                            .call(Edit::RemoveItem {
                                list: list.clone(),
                                name: name.clone(),
                            })
                    }
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
                            let (list, name, tag) = (list.clone(), name.clone(), tag.clone());
                            move |_| {
                                on_edit
                                    .call(Edit::RemoveTag {
                                        list: list.clone(),
                                        name: name.clone(),
                                        tag: tag.clone(),
                                    })
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
                            let (list, name) = (list.clone(), name.clone());
                            move |event: KeyboardEvent| match event.key() {
                                Key::Enter => {
                                    add_tag(on_edit, list.clone(), name.clone(), draft, adding)
                                }
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
