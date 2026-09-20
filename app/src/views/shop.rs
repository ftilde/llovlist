//! Shopping: tick items off one list, nothing else editable.

use std::collections::BTreeSet;

use dioxus::prelude::*;
use llovlist_core::{ItemSnapshot, TrimStr};

use crate::state::{State, edit};

/// Items ticked off in this shop, in tick order. Gone from the data already, kept here so
/// they stay visible greyed out until the shop is over.
#[derive(Clone, PartialEq, Default)]
struct Ticked(Vec<ItemSnapshot>);

impl Ticked {
    fn tick(&mut self, snapshot: ItemSnapshot) {
        self.untick(snapshot.name());
        self.0.push(snapshot);
    }

    fn untick(&mut self, name: &TrimStr) {
        self.0.retain(|ticked| ticked.name() != name);
    }
}

/// Removes the item, remembering how to put it back.
fn tick_off(state: Signal<State>, mut ticked: Signal<Ticked>, list: TrimStr, name: TrimStr) {
    let snapshot = {
        let state = state.read();
        state
            .lists
            .list(&list)
            .snapshot(state.session.device, &name)
    };
    if let Some(snapshot) = snapshot {
        ticked.write().tick(snapshot);
    }
    edit(state, list, |list| list.remove_item(name));
}

fn put_back(state: Signal<State>, mut ticked: Signal<Ticked>, list: TrimStr, name: &TrimStr) {
    let snapshot = ticked
        .read()
        .0
        .iter()
        .find(|ticked| ticked.name() == name)
        .cloned();
    if let Some(snapshot) = snapshot {
        ticked.write().untick(name);
        edit(state, list, |list| list.restore(snapshot));
    }
}

#[component]
pub fn ShopView(list: TrimStr) -> Element {
    let state: Signal<State> = use_context();
    // Owned by the view, so leaving the shop forgets the ticked items with it.
    let ticked = use_signal(Ticked::default);
    let items = state.read().lists.list(&list).items();
    let done: Vec<ItemSnapshot> = ticked
        .read()
        .0
        .iter()
        .filter(|ticked| !items.iter().any(|(live, _)| live == ticked.name()))
        .cloned()
        .collect();

    rsx! {
        main { class: "page",
            ul { class: "items",
                for (name , tags) in items.clone() {
                    ShopRow {
                        key: "{name}",
                        name: name.clone(),
                        tags,
                        done: false,
                        on_tick: {
                            let list = list.clone();
                            move |name: TrimStr| tick_off(state, ticked, list.clone(), name)
                        },
                    }
                }
                for snapshot in done {
                    ShopRow {
                        key: "done-{snapshot.name()}",
                        name: snapshot.name().clone(),
                        tags: snapshot.tags().clone(),
                        done: true,
                        on_tick: {
                            let list = list.clone();
                            move |name: TrimStr| put_back(state, ticked, list.clone(), &name)
                        },
                    }
                }
            }
            if items.is_empty() {
                p { class: "empty", "all done" }
            }
        }
    }
}

/// An item while shopping: tick it off, read its tags, nothing else.
#[component]
fn ShopRow(
    name: TrimStr,
    tags: BTreeSet<TrimStr>,
    done: bool,
    on_tick: EventHandler<TrimStr>,
) -> Element {
    rsx! {
        li { class: if done { "item done" } else { "item" },
            button {
                class: "tick",
                title: if done { "put it back" } else { "got it" },
                onclick: {
                    let name = name.clone();
                    move |_| on_tick.call(name.clone())
                },
                if done { "✓" } else { "○" }
            }
            span { class: "name", "{name}" }
            div { class: "tags",
                for tag in tags.iter() {
                    span { key: "{tag}", class: "tag", "{tag}" }
                }
            }
        }
    }
}
