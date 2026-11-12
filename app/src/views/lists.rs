//! Editing: pick a list, add, tag, tick and reorder its items.

use std::collections::BTreeSet;

use dioxus::prelude::*;
use llovlist_core::TrimStr;

use crate::drag::{Drag, DragCatcher, Grip, row_class};
use crate::state::{Scope, State, add_item, add_tag, edit, move_item_to, move_list_to};

#[component]
pub fn ListsView(
    scope: Scope,
    selected: Option<TrimStr>,
    current: Signal<Option<TrimStr>>,
) -> Element {
    let state: Signal<State> = use_context();
    let mut current = current;
    let mut draft = use_signal(String::new);
    let list_drag = use_signal(|| None::<Drag>);
    let item_drag = use_signal(|| None::<Drag>);

    let list_line = list_drag().map(|drag| drag.line_before());
    let item_line = item_drag().map(|drag| drag.line_before());
    let names = state.read().names(scope);
    let items = selected
        .as_ref()
        .map(|list| scope.of(&state.read().data).list(list).items())
        .unwrap_or_default();
    let suggestions: Vec<TrimStr> = match &selected {
        Some(list) if !draft.read().trim().is_empty() => state
            .read()
            .data
            .usage
            .suggest_items(list, &draft.read())
            .into_iter()
            .filter(|name| !items.iter().any(|(listed, _)| listed == name))
            .collect(),
        _ => Vec::new(),
    };

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
                            move |slot| move_list_to(state, scope, &name, slot)
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
                p { class: "empty", "nothing here yet — add one from the menu" }
            }
        }

        main { class: "page",
            if let Some(list) = selected.clone() {
                if items.is_empty() {
                    p { class: "empty", "no items yet" }
                }
                ul { class: "items",
                    for (index , (name , tags)) in items.clone().into_iter().enumerate() {
                        ItemRow {
                            key: "{name}",
                            index,
                            count: items.len(),
                            line: item_line,
                            scope,
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
            if !suggestions.is_empty() {
                div { class: "suggestions item-suggestions",
                    for name in suggestions {
                        button {
                            key: "{name}",
                            class: "tag",
                            onclick: {
                                let list = list.clone();
                                move |_| {
                                    add_item(state, scope, list.clone(), name.clone());
                                    draft.set(String::new());
                                }
                            },
                            "{name}"
                        }
                    }
                }
            }
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
                                submit_item(state, scope, list.clone(), draft);
                            }
                        }
                    },
                }
                button {
                    class: "icon",
                    onclick: {
                        let list = list.clone();
                        move |_| submit_item(state, scope, list.clone(), draft)
                    },
                    "+"
                }
            }
        }

        if let Some(dragged) = list_drag().and_then(|drag| names.get(drag.from).cloned()) {
            DragCatcher {
                drag: list_drag,
                on_drop: move |slot| move_list_to(state, scope, &dragged, slot),
            }
        }

        if let Some((list, dragged)) = selected
            .clone()
            .zip(item_drag().and_then(|drag| items.get(drag.from).cloned()))
        {
            DragCatcher {
                drag: item_drag,
                on_drop: move |slot| move_item_to(state, scope, &list, &dragged.0, slot),
            }
        }
    }
}

fn submit_item(state: Signal<State>, scope: Scope, list: TrimStr, mut draft: Signal<String>) {
    let name = TrimStr::new(&draft.read());
    if let Ok(name) = name {
        add_item(state, scope, list, name);
        draft.set(String::new());
    }
}

fn submit_tag(
    state: Signal<State>,
    scope: Scope,
    list: TrimStr,
    name: TrimStr,
    mut draft: Signal<String>,
    mut adding: Signal<bool>,
) {
    let tag = TrimStr::new(&draft.read());
    if let Ok(tag) = tag {
        add_tag(state, scope, list, name, tag);
    }
    draft.set(String::new());
    adding.set(false);
}

#[component]
fn ItemRow(
    index: usize,
    count: usize,
    line: Option<usize>,
    scope: Scope,
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
    let suggestions: Vec<TrimStr> = if adding() {
        let usage = &state.read().data.usage;
        let known = usage.suggest_tags(&name, &draft.read());
        known
            .into_iter()
            .filter(|tag| !tags.contains(tag))
            .collect()
    } else {
        Vec::new()
    };

    rsx! {
        li { class: row_class(base, line, index, count),
            Grip {
                index,
                selector: ".items > .item",
                drag,
                on_drop: {
                    let (list, name) = (list.clone(), name.clone());
                    move |slot| move_item_to(state, scope, &list, &name, slot)
                },
            }
            button {
                class: "tick",
                title: "tick off",
                onclick: {
                    let (list, name) = (list.clone(), name.clone());
                    move |_| {
                        let name = name.clone();
                        edit(state, scope, list.clone(), |list| list.remove_item(name))
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
                                edit(state, scope, list.clone(), |list| list.remove_tag(name, tag))
                            }
                        },
                        "{tag} ×"
                    }
                }
                if adding() {
                    input {
                        class: "tag-input",
                        placeholder: "2x",
                        // The webview honours `autofocus` once per page, not on every insertion.
                        onmounted: move |mounted: MountedEvent| async move {
                            let _ = mounted.set_focus(true).await;
                        },
                        value: "{draft}",
                        enterkeyhint: "done",
                        oninput: move |event| draft.set(event.value()),
                        onkeydown: {
                            let (list, name) = (list.clone(), name.clone());
                            move |event: KeyboardEvent| match event.key() {
                                Key::Enter => {
                                    submit_tag(state, scope, list.clone(), name.clone(), draft, adding)
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
            if !suggestions.is_empty() {
                div { class: "suggestions tag-suggestions",
                    for tag in suggestions {
                        button {
                            key: "{tag}",
                            class: "add-tag",
                            onclick: {
                                let (list, name) = (list.clone(), name.clone());
                                move |_| {
                                    add_tag(state, scope, list.clone(), name.clone(), tag.clone());
                                    draft.set(String::new());
                                    adding.set(false);
                                }
                            },
                            "{tag}"
                        }
                    }
                }
            }
        }
    }
}
