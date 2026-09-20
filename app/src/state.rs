//! The device's data, who edits it, and how it is persisted.

use dioxus::prelude::*;
use llovlist_core::{DeviceId, ListMut, Lists, Position, Session, TrimStr};
use serde::{Deserialize, Serialize};

use crate::storage;

const LISTS_KEY: &str = "lists.json";
const SETTINGS_KEY: &str = "settings.json";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub server: String,
    pub token: String,
    pub device: DeviceId,
    /// The list being shopped, so a relaunch lands back in the shop without unlocking.
    #[serde(default)]
    pub shopping: Option<TrimStr>,
}

impl Settings {
    pub fn load() -> Self {
        storage::load(SETTINGS_KEY)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or(Self {
                server: String::new(),
                token: String::new(),
                device: DeviceId::new(storage::random_u64()),
                shopping: None,
            })
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string(self) {
            storage::save(SETTINGS_KEY, &json);
        }
    }
}

/// The device's data plus who is editing it.
pub struct State {
    pub session: Session,
    pub lists: Lists,
}

impl State {
    pub fn load(device: DeviceId) -> Self {
        Self {
            session: Session::new(device),
            lists: storage::load(LISTS_KEY)
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default(),
        }
    }
}

pub fn store(state: Signal<State>) {
    if let Ok(json) = serde_json::to_string(&state.read().lists) {
        storage::save(LISTS_KEY, &json);
    }
}

/// Runs `f` against the lists and persists the result.
pub fn change(mut state: Signal<State>, f: impl FnOnce(&Session, &mut Lists)) {
    {
        let mut state = state.write();
        let State { session, lists } = &mut *state;
        f(session, lists);
    }
    store(state);
}

/// Runs `f` against `list` and persists the result.
pub fn edit(state: Signal<State>, list: TrimStr, f: impl FnOnce(&mut ListMut)) {
    change(
        state,
        |session, lists| f(&mut lists.list_mut(session, list)),
    );
}

pub fn create_list(state: Signal<State>, list: TrimStr) {
    change(state, |session, lists| {
        lists.create(session, list, Position::End)
    });
}

pub fn remove_list(state: Signal<State>, list: &TrimStr) {
    change(state, |session, lists| lists.remove(session, list));
}

pub fn restore_deleted(state: Signal<State>) {
    change(state, |session, lists| lists.restore_deleted(session));
}

pub fn move_list_to(state: Signal<State>, list: &TrimStr, slot: usize) {
    edit(state, list.clone(), |list| list.move_to_slot(slot));
}

pub fn move_item_to(state: Signal<State>, list: &TrimStr, name: &TrimStr, slot: usize) {
    let name = name.clone();
    edit(state, list.clone(), |list| {
        list.move_item_to_slot(name, slot)
    });
}
