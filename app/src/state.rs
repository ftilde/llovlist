//! The device's data, who edits it, and how it is persisted.

use dioxus::prelude::*;
use llovlist_core::{Data, DeviceId, ListMut, Lists, Position, Session, TrimStr};
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
    pub data: Data,
}

impl State {
    pub fn load(device: DeviceId) -> Self {
        Self {
            session: Session::new(device),
            data: storage::load(LISTS_KEY)
                .and_then(|json| Data::from_json(&json).ok())
                .unwrap_or_default(),
        }
    }
}

/// Which collection an edit or view is about: the lists themselves, or the templates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Lists,
    Templates,
}

impl Scope {
    pub fn of(self, data: &Data) -> &Lists {
        match self {
            Self::Lists => &data.lists,
            Self::Templates => &data.templates,
        }
    }

    fn of_mut(self, data: &mut Data) -> &mut Lists {
        match self {
            Self::Lists => &mut data.lists,
            Self::Templates => &mut data.templates,
        }
    }
}

pub fn store(state: Signal<State>) {
    if let Ok(json) = serde_json::to_string(&state.read().data) {
        storage::save(LISTS_KEY, &json);
    }
}

/// Runs `f` against the data and persists the result.
pub fn change(mut state: Signal<State>, f: impl FnOnce(&Session, &mut Data)) {
    {
        let mut state = state.write();
        let State { session, data } = &mut *state;
        f(session, data);
    }
    store(state);
}

/// Runs `f` against `list` in `scope` and persists the result.
pub fn edit(state: Signal<State>, scope: Scope, list: TrimStr, f: impl FnOnce(&mut ListMut)) {
    change(state, |session, data| {
        f(&mut scope.of_mut(data).list_mut(session, list))
    });
}

pub fn create_list(state: Signal<State>, scope: Scope, list: TrimStr) {
    change(state, |session, data| {
        scope.of_mut(data).create(session, list, Position::End)
    });
}

pub fn remove_list(state: Signal<State>, scope: Scope, list: &TrimStr) {
    change(state, |session, data| {
        scope.of_mut(data).remove(session, list)
    });
}

pub fn restore_deleted(state: Signal<State>, scope: Scope) {
    change(state, |session, data| {
        scope.of_mut(data).restore_deleted(session)
    });
}

pub fn move_list_to(state: Signal<State>, scope: Scope, list: &TrimStr, slot: usize) {
    edit(state, scope, list.clone(), |list| list.move_to_slot(slot));
}

pub fn move_item_to(
    state: Signal<State>,
    scope: Scope,
    list: &TrimStr,
    name: &TrimStr,
    slot: usize,
) {
    let name = name.clone();
    edit(state, scope, list.clone(), |list| {
        list.move_item_to_slot(name, slot)
    });
}

/// Adds every item of `template` to `list`, one undo step.
pub fn fill_from(state: Signal<State>, list: TrimStr, template: &TrimStr) {
    change(state, |session, data| {
        let template = data.templates.list(template);
        data.lists.list_mut(session, list).fill_from(&template);
    });
}
