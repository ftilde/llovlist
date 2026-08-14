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

fn undo(mut session: Signal<Session>, list: &ListName) {
    session.write().undo(list);
    store(session);
}

fn restore_deleted(mut session: Signal<Session>) {
    session.write().restore_deleted();
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

/// A reorder in progress: the row picked up, and where it would land.
#[derive(Clone, PartialEq)]
struct Drag {
    /// Position of the dragged row in the displayed order.
    from: usize,
    /// Midpoint of every row, in display order, as of pick-up.
    midpoints: Vec<f64>,
    slot: usize,
}

impl Drag {
    fn aim(&mut self, y: f64) {
        if let Some(slot) = slot_under(self.from, &self.midpoints, y) {
            self.slot = slot;
        }
    }

    /// Whether the rows have been measured, so a release means anything yet.
    fn aimed(&self) -> bool {
        !self.midpoints.is_empty()
    }

    /// Where the insertion line sits: the display index the row would land before.
    fn line_before(&self) -> usize {
        if self.slot >= self.from {
            self.slot + 1
        } else {
            self.slot
        }
    }
}

/// Marks where the insertion line goes. A shadow rather than a border, so showing it
/// does not shift the rows the pointer is being aimed at.
fn row_class(base: &str, line: Option<usize>, index: usize, count: usize) -> String {
    let mut class = base.to_owned();
    if line == Some(index) {
        class.push_str(" drop-above");
    }
    if line == Some(count) && index + 1 == count {
        class.push_str(" drop-below");
    }
    class
}

/// Slot in the order without the dragged row, or `None` before the rows are measured --
/// an empty set of midpoints would otherwise read as "above everything".
fn slot_under(from: usize, midpoints: &[f64], y: f64) -> Option<usize> {
    if midpoints.is_empty() {
        return None;
    }
    let insertion = midpoints.iter().filter(|midpoint| y > **midpoint).count();
    // Taking the dragged row out shifts everything after it down by one.
    Some(if insertion > from {
        insertion - 1
    } else {
        insertion
    })
}

/// Midpoints of the rows matching `selector`, measured in the webview.
async fn row_midpoints(selector: &str) -> Vec<f64> {
    let script = format!(
        "dioxus.send(Array.from(document.querySelectorAll('{selector}')).map((row) => {{
            const box = row.getBoundingClientRect();
            return box.top + box.height / 2;
        }}));"
    );
    document::eval(&script).recv().await.unwrap_or_default()
}

fn aim_drag(mut drag: Signal<Option<Drag>>, event: &PointerEvent) {
    if let Some(mut active) = drag() {
        active.aim(event.data().client_coordinates().y);
        drag.set(Some(active));
    }
}

fn finish_drag(mut drag: Signal<Option<Drag>>, on_drop: EventHandler<usize>) {
    if let Some(active) = drag.take().filter(Drag::aimed) {
        on_drop.call(active.slot);
    }
}

/// Catches move and release while dragging with a mouse. Touch pointers stay captured by
/// the grip, but mouse pointers retarget to whatever is under the cursor, so without this
/// a release away from the grip is never seen.
#[component]
fn DragCatcher(drag: Signal<Option<Drag>>, on_drop: EventHandler<usize>) -> Element {
    rsx! {
        div {
            class: "drag-catcher",
            onpointermove: move |event: PointerEvent| aim_drag(drag, &event),
            onpointerup: move |_| finish_drag(drag, on_drop),
            onpointercancel: move |_| drag.set(None),
        }
    }
}

/// The handle a row is dragged by. Pointer events during a touch drag all go to the
/// element the gesture started on, so every handler belongs here rather than on the list.
#[component]
fn Grip(
    index: usize,
    selector: String,
    drag: Signal<Option<Drag>>,
    on_drop: EventHandler<usize>,
) -> Element {
    let mut drag = drag;
    rsx! {
        button {
            class: "grip",
            title: "drag to reorder",
            onpointerdown: move |event: PointerEvent| {
                // Set synchronously, so a release that beats the measurement below still
                // finds a drag to cancel rather than silently doing nothing.
                drag.set(Some(Drag {
                    from: index,
                    midpoints: Vec::new(),
                    slot: index,
                }));
                let y = event.data().client_coordinates().y;
                let selector = selector.clone();
                spawn(async move {
                    let midpoints = row_midpoints(&selector).await;
                    if let Some(mut active) = drag() {
                        active.midpoints = midpoints;
                        active.aim(y);
                        drag.set(Some(active));
                    }
                });
            },
            onpointermove: move |event: PointerEvent| aim_drag(drag, &event),
            onpointerup: move |_| finish_drag(drag, on_drop),
            onpointercancel: move |_| drag.set(None),
            "⠿"
        }
    }
}

fn move_list_to(session: Signal<Session>, list: &ListName, slot: usize) {
    let key = session.read().key_for_slot(list, slot);
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

fn move_item_to(session: Signal<Session>, list: &ListName, name: &ItemName, slot: usize) {
    let key = session.read().item_key_for_slot(list, name, slot);
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
    let list_drag = use_signal(|| None::<Drag>);
    let item_drag = use_signal(|| None::<Drag>);

    let list_line = list_drag().map(|drag| drag.line_before());
    let item_line = item_drag().map(|drag| drag.line_before());
    let names = session.read().lists().names();
    let can_restore = session.read().can_restore();
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
    let can_undo = selected
        .as_ref()
        .is_some_and(|list| session.read().can_undo(list));

    rsx! {
        document::Style { {CSS} }
        header {
            h1 { "1kauf" }
            button {
                disabled: !can_undo,
                onclick: {
                    let selected = selected.clone();
                    move |_| {
                        if let Some(list) = selected.clone() {
                            undo(session, &list);
                        }
                    }
                },
                "undo"
            }
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
                for (index , name) in names.clone().into_iter().enumerate() {
                    div {
                        key: "{name}",
                        class: row_class("tab-row", list_line, index, names.len()),
                        Grip {
                            index,
                            selector: ".tabs > .tab-row",
                            drag: list_drag,
                            on_drop: {
                                let name = name.clone();
                                move |slot| move_list_to(session, &name, slot)
                            },
                        }
                        button {
                            class: if Some(&name) == selected.as_ref() { "tab active" } else { "tab" },
                            onclick: {
                                let name = name.clone();
                                move |_| current.set(Some(name.clone()))
                            },
                            "{name}"
                        }
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
                if can_restore {
                    button {
                        class: "restore",
                        onclick: move |_| restore_deleted(session),
                        "restore list"
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
                        for (index , (name , tags)) in items.clone().into_iter().enumerate() {
                            ItemRow {
                                key: "{name}",
                                index,
                                count: items.len(),
                                line: item_line,
                                list: list.clone(),
                                name,
                                tags,
                                session,
                                drag: item_drag,
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

        if let Some(dragged) = list_drag().and_then(|drag| names.get(drag.from).cloned()) {
            DragCatcher {
                drag: list_drag,
                on_drop: move |slot| move_list_to(session, &dragged, slot),
            }
        }

        if let Some((list, dragged)) = selected
            .clone()
            .zip(item_drag().and_then(|drag| items.get(drag.from).cloned()))
        {
            DragCatcher {
                drag: item_drag,
                on_drop: move |slot| move_item_to(session, &list, &dragged.0, slot),
            }
        }
    }
}

#[component]
fn ItemRow(
    index: usize,
    count: usize,
    line: Option<usize>,
    list: ListName,
    name: ItemName,
    tags: BTreeSet<Tag>,
    session: Signal<Session>,
    drag: Signal<Option<Drag>>,
    on_edit: EventHandler<Edit>,
) -> Element {
    let mut adding = use_signal(|| false);
    let mut draft = use_signal(String::new);
    let base = if drag().is_some_and(|active| active.from == index) {
        "item dragged"
    } else {
        "item"
    };

    rsx! {
        li { class: row_class(base, line, index, count),
            Grip {
                index,
                selector: ".items > .item",
                drag,
                on_drop: {
                    let (list, name) = (list.clone(), name.clone());
                    move |slot| move_item_to(session, &list, &name, slot)
                },
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

#[cfg(test)]
mod tests {
    use super::*;

    // Three rows 20px tall starting at y=0.
    const MIDPOINTS: [f64; 3] = [10.0, 30.0, 50.0];

    #[test]
    fn dragging_the_first_row_downwards_walks_through_every_slot() {
        assert_eq!(slot_under(0, &MIDPOINTS, 5.0), Some(0));
        assert_eq!(slot_under(0, &MIDPOINTS, 15.0), Some(0));
        assert_eq!(slot_under(0, &MIDPOINTS, 35.0), Some(1));
        assert_eq!(slot_under(0, &MIDPOINTS, 55.0), Some(2));
    }

    #[test]
    fn dragging_the_last_row_upwards_walks_through_every_slot() {
        assert_eq!(slot_under(2, &MIDPOINTS, 5.0), Some(0));
        assert_eq!(slot_under(2, &MIDPOINTS, 25.0), Some(1));
        assert_eq!(slot_under(2, &MIDPOINTS, 55.0), Some(2));
    }

    // Above its own midpoint and just below it are both "stay put".
    #[test]
    fn a_row_dropped_on_itself_keeps_its_slot() {
        assert_eq!(slot_under(1, &MIDPOINTS, 25.0), Some(1));
        assert_eq!(slot_under(1, &MIDPOINTS, 35.0), Some(1));
    }

    // A release before the rows are measured must cancel, not move to the top.
    #[test]
    fn an_unmeasured_drag_has_no_slot_and_does_not_fire() {
        assert_eq!(slot_under(1, &[], 25.0), None);

        let mut drag = Drag {
            from: 1,
            midpoints: Vec::new(),
            slot: 1,
        };
        drag.aim(5.0);
        assert_eq!(drag.slot, 1);
        assert!(!drag.aimed());
    }

    #[test]
    fn the_line_sits_where_the_row_would_land() {
        let mut drag = Drag {
            from: 0,
            midpoints: MIDPOINTS.to_vec(),
            slot: 0,
        };

        drag.aim(5.0);
        assert_eq!(drag.line_before(), 1);
        drag.aim(35.0);
        assert_eq!(drag.line_before(), 2);
        drag.aim(55.0);
        assert_eq!(drag.line_before(), 3);

        drag.from = 2;
        drag.aim(5.0);
        assert_eq!(drag.line_before(), 0);
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
