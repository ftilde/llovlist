mod storage;
mod sync;

use std::collections::BTreeSet;

use dioxus::prelude::*;
use llovlist_core::{DeviceId, ListMut, Lists, Position, Session, TrimStr};
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
            .with_title("llovlist");
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

/// The device's data plus who is editing it.
struct State {
    session: Session,
    lists: Lists,
}

fn store(state: Signal<State>) {
    if let Ok(json) = serde_json::to_string(&state.read().lists) {
        storage::save(LISTS_KEY, &json);
    }
}

/// Runs `f` against `list` and persists the result.
fn edit(mut state: Signal<State>, list: TrimStr, f: impl FnOnce(&mut ListMut)) {
    {
        let mut state = state.write();
        let State { session, lists } = &mut *state;
        f(&mut lists.list_mut(session, list));
    }
    store(state);
}

fn remove_list(mut state: Signal<State>, list: &TrimStr) {
    {
        let mut state = state.write();
        let State { session, lists } = &mut *state;
        lists.remove(session, list);
    }
    store(state);
}

fn restore_deleted(mut state: Signal<State>) {
    {
        let mut state = state.write();
        let State { session, lists } = &mut *state;
        lists.restore_deleted(session);
    }
    store(state);
}

fn add_item(state: Signal<State>, list: TrimStr, mut draft: Signal<String>) {
    let name = TrimStr::new(&draft.read());
    if let Ok(name) = name {
        edit(state, list, |list| list.add_item(name, Position::End));
        draft.set(String::new());
    }
}

fn create_list(
    mut state: Signal<State>,
    mut current: Signal<Option<TrimStr>>,
    mut draft: Signal<String>,
) {
    let list = TrimStr::new(&draft.read());
    if let Ok(list) = list {
        {
            let mut state = state.write();
            let State { session, lists } = &mut *state;
            lists.create(session, list.clone(), Position::End);
        }
        store(state);
        current.set(Some(list));
        draft.set(String::new());
    }
}

/// A reorder in progress: the row picked up, and where it would land.
#[derive(Clone, PartialEq)]
struct Drag {
    /// Position of the dragged row in the displayed order.
    from: usize,
    /// Midpoint of every row along the drag axis, in display order, as of pick-up.
    midpoints: Vec<f64>,
    slot: usize,
    horizontal: bool,
}

impl Drag {
    fn aim(&mut self, along: f64) {
        if let Some(slot) = slot_under(self.from, &self.midpoints, along) {
            self.slot = slot;
        }
    }

    /// The pointer's coordinate along the drag axis.
    fn along(&self, event: &PointerEvent) -> f64 {
        let point = event.data().client_coordinates();
        if self.horizontal { point.x } else { point.y }
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
async fn row_midpoints(selector: &str, horizontal: bool) -> Vec<f64> {
    let (start, size) = if horizontal {
        ("left", "width")
    } else {
        ("top", "height")
    };
    let script = format!(
        "dioxus.send(Array.from(document.querySelectorAll('{selector}')).map((row) => {{
            const box = row.getBoundingClientRect();
            return box.{start} + box.{size} / 2;
        }}));"
    );
    document::eval(&script).recv().await.unwrap_or_default()
}

fn aim_drag(mut drag: Signal<Option<Drag>>, event: &PointerEvent) {
    if let Some(mut active) = drag() {
        let along = active.along(event);
        active.aim(along);
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
    #[props(default)] horizontal: bool,
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
                let fresh = Drag {
                    from: index,
                    midpoints: Vec::new(),
                    slot: index,
                    horizontal,
                };
                let along = fresh.along(&event);
                drag.set(Some(fresh));
                let selector = selector.clone();
                spawn(async move {
                    let midpoints = row_midpoints(&selector, horizontal).await;
                    if let Some(mut active) = drag() {
                        active.midpoints = midpoints;
                        active.aim(along);
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

fn move_list_to(state: Signal<State>, list: &TrimStr, slot: usize) {
    edit(state, list.clone(), |list| list.move_to_slot(slot));
}

fn move_item_to(state: Signal<State>, list: &TrimStr, name: &TrimStr, slot: usize) {
    let name = name.clone();
    edit(state, list.clone(), |list| {
        list.move_item_to_slot(name, slot)
    });
}

fn add_tag(
    state: Signal<State>,
    list: TrimStr,
    name: TrimStr,
    mut draft: Signal<String>,
    mut adding: Signal<bool>,
) {
    let tag = TrimStr::new(&draft.read());
    if let Ok(tag) = tag {
        edit(state, list, |list| list.add_tag(name, tag));
    }
    draft.set(String::new());
    adding.set(false);
}

fn start_sync(mut state: Signal<State>, settings: Signal<Settings>, notice: Signal<Notice>) {
    let settings = settings.read().clone();
    let local = state.read().lists.clone();
    spawn(async move {
        match sync::exchange(&settings.server, &settings.token, local).await {
            Ok(merged) => {
                state.write().lists.merge(merged);
                store(state);
                let count = state.read().lists.names().len();
                let plural = if count == 1 { "" } else { "s" };
                show(notice, format!("synced, {count} list{plural}"));
            }
            Err(reason) => {
                eprintln!("sync failed: {reason}");
                show(notice, format!("sync failed: {reason}"));
            }
        }
    });
}

/// A short message at the bottom of the screen that goes away on its own.
#[derive(Clone, PartialEq, Default)]
struct Notice {
    text: String,
    // Distinguishes a newer notice from the one whose timer is running out.
    serial: u64,
}

fn show(mut notice: Signal<Notice>, text: String) {
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

#[derive(Clone, Copy, PartialEq)]
enum View {
    Lists,
    Settings,
}

#[component]
fn App() -> Element {
    let settings = use_signal(Settings::load);
    let state = use_signal(|| State {
        session: Session::new(settings.read().device),
        lists: load_lists(),
    });
    let notice = use_signal(Notice::default);
    let mut view = use_signal(|| View::Lists);
    let mut menu_open = use_signal(|| false);
    let mut adding_list = use_signal(|| false);
    let mut current = use_signal(|| state.read().lists.names().first().cloned());
    let mut draft = use_signal(String::new);
    let mut new_list = use_signal(String::new);
    let list_drag = use_signal(|| None::<Drag>);
    let item_drag = use_signal(|| None::<Drag>);

    let list_line = list_drag().map(|drag| drag.line_before());
    let item_line = item_drag().map(|drag| drag.line_before());
    let names = state.read().lists.names();
    let restorable = state.read().lists.restorable().cloned();
    let message = notice.read().text.clone();

    // Falling back to the first name keeps a list selected when the current one is
    // deleted here or disappears in a merge.
    let selected = current()
        .filter(|list| names.contains(list))
        .or_else(|| names.first().cloned());
    let items = selected
        .as_ref()
        .map(|list| state.read().lists.list(list).items())
        .unwrap_or_default();
    let can_undo = selected
        .as_ref()
        .is_some_and(|list| state.read().lists.can_undo(list));
    let can_redo = selected
        .as_ref()
        .is_some_and(|list| state.read().lists.can_redo(list));

    let for_selected = |f: fn(&mut ListMut)| {
        let selected = selected.clone();
        move |_| {
            if let Some(list) = selected.clone() {
                edit(state, list, f);
            }
        }
    };

    rsx! {
        document::Style { {CSS} }
        div { class: "app",
            header { class: "topbar",
                if view() == View::Settings {
                    button { class: "icon", title: "back", onclick: move |_| view.set(View::Lists), "←" }
                    h1 { "settings" }
                } else {
                    h1 { "llovlist" }
                    button { class: "icon", title: "undo", disabled: !can_undo, onclick: for_selected(|list| list.undo()), "↶" }
                    button { class: "icon", title: "redo", disabled: !can_redo, onclick: for_selected(|list| list.redo()), "↷" }
                    button { class: "icon", title: "sync", onclick: move |_| start_sync(state, settings, notice), "⟳" }
                    button {
                        class: if menu_open() { "icon active" } else { "icon" },
                        title: "menu",
                        onclick: move |_| menu_open.toggle(),
                        "☰"
                    }
                }
            }

            if menu_open() {
                div { class: "scrim", onclick: move |_| menu_open.set(false) }
                nav { class: "menu",
                    button { onclick: move |_| { menu_open.set(false); adding_list.set(true); }, "add list" }
                    if let Some(list) = selected.clone() {
                        button {
                            onclick: move |_| { menu_open.set(false); remove_list(state, &list); },
                            "delete list “{list}”"
                        }
                    }
                    if let Some(list) = restorable {
                        button {
                            onclick: move |_| { menu_open.set(false); restore_deleted(state); },
                            "restore list “{list}”"
                        }
                    }
                    button { onclick: move |_| { menu_open.set(false); view.set(View::Settings); }, "settings" }
                }
            }

            if view() == View::Settings {
                main { class: "page", SettingsPanel { settings } }
            } else {
                nav { class: "tabs",
                    for (index , name) in names.clone().into_iter().enumerate() {
                        div {
                            key: "{name}",
                            class: row_class("tab-row", list_line, index, names.len()),
                            Grip {
                                index,
                                selector: ".tabs > .tab-row",
                                horizontal: true,
                                drag: list_drag,
                                on_drop: {
                                    let name = name.clone();
                                    move |slot| move_list_to(state, &name, slot)
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
                    if names.is_empty() {
                        p { class: "empty", "no list yet — add one from the menu" }
                    }
                }

                main { class: "page",
                    if let Some(list) = selected.clone() {
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
                                    state,
                                    drag: item_drag,
                                }
                            }
                        }
                    }
                }

                if let Some(list) = selected.clone() {
                    footer { class: "add",
                        input {
                            placeholder: "add an item",
                            value: "{draft}",
                            enterkeyhint: "done",
                            oninput: move |event| draft.set(event.value()),
                            onkeydown: {
                                let list = list.clone();
                                move |event: KeyboardEvent| {
                                    if event.key() == Key::Enter {
                                        add_item(state, list.clone(), draft);
                                    }
                                }
                            },
                        }
                        button {
                            class: "icon",
                            onclick: {
                                let list = list.clone();
                                move |_| add_item(state, list.clone(), draft)
                            },
                            "+"
                        }
                    }
                }
            }

            if !message.is_empty() {
                p { class: "notice", "{message}" }
            }
        }

        if adding_list() {
            div { class: "overlay", onclick: move |_| adding_list.set(false),
                div { class: "card", onclick: move |event| event.stop_propagation(),
                    h2 { "new list" }
                    input {
                        placeholder: "name",
                        autofocus: true,
                        value: "{new_list}",
                        // Without this Android offers Next, which moves focus to the next
                        // field instead of sending Enter.
                        enterkeyhint: "done",
                        oninput: move |event| new_list.set(event.value()),
                        onkeydown: move |event: KeyboardEvent| match event.key() {
                            Key::Enter => {
                                create_list(state, current, new_list);
                                adding_list.set(false);
                            }
                            Key::Escape => adding_list.set(false),
                            _ => {}
                        },
                    }
                    div { class: "actions",
                        button { onclick: move |_| adding_list.set(false), "cancel" }
                        button {
                            class: "primary",
                            onclick: move |_| {
                                create_list(state, current, new_list);
                                adding_list.set(false);
                            },
                            "add"
                        }
                    }
                }
            }
        }

        if let Some(dragged) = list_drag().and_then(|drag| names.get(drag.from).cloned()) {
            DragCatcher {
                drag: list_drag,
                on_drop: move |slot| move_list_to(state, &dragged, slot),
            }
        }

        if let Some((list, dragged)) = selected
            .clone()
            .zip(item_drag().and_then(|drag| items.get(drag.from).cloned()))
        {
            DragCatcher {
                drag: item_drag,
                on_drop: move |slot| move_item_to(state, &list, &dragged.0, slot),
            }
        }
    }
}

#[component]
fn ItemRow(
    index: usize,
    count: usize,
    line: Option<usize>,
    list: TrimStr,
    name: TrimStr,
    tags: BTreeSet<TrimStr>,
    state: Signal<State>,
    drag: Signal<Option<Drag>>,
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
                    move |slot| move_item_to(state, &list, &name, slot)
                },
            }
            button {
                class: "tick",
                title: "got it",
                onclick: {
                    let (list, name) = (list.clone(), name.clone());
                    move |_| {
                        let name = name.clone();
                        edit(state, list.clone(), |list| list.remove_item(name))
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
                                let (name, tag) = (name.clone(), tag.clone());
                                edit(state, list.clone(), |list| list.remove_tag(name, tag))
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
                        enterkeyhint: "done",
                        oninput: move |event| draft.set(event.value()),
                        onkeydown: {
                            let (list, name) = (list.clone(), name.clone());
                            move |event: KeyboardEvent| match event.key() {
                                Key::Enter => {
                                    add_tag(state, list.clone(), name.clone(), draft, adding)
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
            horizontal: false,
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
            horizontal: false,
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
