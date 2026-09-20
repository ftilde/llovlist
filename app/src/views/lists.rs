//! Editing: pick a list, add, tag, tick and reorder its items.

use std::collections::BTreeSet;

use dioxus::prelude::*;
use llovlist_core::{Position, TrimStr};

use crate::drag::{Drag, DragCatcher, Grip, row_class};
use crate::state::{State, edit, move_item_to, move_list_to};

#[component]
pub fn ListsView(selected: Option<TrimStr>, current: Signal<Option<TrimStr>>) -> Element {
    let state: Signal<State> = use_context();
    let mut current = current;
    let mut draft = use_signal(String::new);
    let list_drag = use_signal(|| None::<Drag>);
    let item_drag = use_signal(|| None::<Drag>);

    let list_line = list_drag().map(|drag| drag.line_before());
    let item_line = item_drag().map(|drag| drag.line_before());
    let names = state.read().lists.names();
    let items = selected
        .as_ref()
        .map(|list| state.read().lists.list(list).items())
        .unwrap_or_default();

    rsx! {
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

fn add_item(state: Signal<State>, list: TrimStr, mut draft: Signal<String>) {
    let name = TrimStr::new(&draft.read());
    if let Ok(name) = name {
        edit(state, list, |list| list.add_item(name, Position::End));
        draft.set(String::new());
    }
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

#[component]
fn ItemRow(
    index: usize,
    count: usize,
    line: Option<usize>,
    list: TrimStr,
    name: TrimStr,
    tags: BTreeSet<TrimStr>,
    drag: Signal<Option<Drag>>,
) -> Element {
    let state: Signal<State> = use_context();
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
