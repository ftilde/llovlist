use std::collections::BTreeSet;
use std::fmt;

use crdts::{CmRDT, CvRDT, Map, Orswot};
use serde::{Deserialize, Serialize};

// Two replicas sharing an id mint the same dots and silently drop each other's entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeviceId(u64);

impl DeviceId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Blank;

impl fmt::Display for Blank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("must not be blank")
    }
}

impl std::error::Error for Blank {}

fn trimmed(raw: &str) -> Result<String, Blank> {
    let trimmed = raw.trim();
    if trimmed.is_empty() { Err(Blank) } else { Ok(trimmed.to_owned()) }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ItemName(String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Tag(String);

macro_rules! trimmed_newtype {
    ($type:ident) => {
        impl $type {
            pub fn new(raw: &str) -> Result<Self, Blank> {
                trimmed(raw).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $type {
            type Error = Blank;

            fn try_from(raw: String) -> Result<Self, Blank> {
                Self::new(&raw)
            }
        }

        impl From<$type> for String {
            fn from(value: $type) -> Self {
                value.0
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

trimmed_newtype!(ItemName);
trimmed_newtype!(Tag);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    AddItem { name: ItemName, tags: BTreeSet<Tag> },
    RemoveItem { name: ItemName },
    AddTag { name: ItemName, tag: Tag },
    RemoveTag { name: ItemName, tag: Tag },
}

impl Edit {
    pub fn add_item(name: ItemName) -> Self {
        Self::AddItem { name, tags: BTreeSet::new() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShoppingList {
    items: Map<ItemName, Orswot<Tag, DeviceId>, DeviceId>,
}

impl ShoppingList {
    pub fn items(&self) -> Vec<(ItemName, BTreeSet<Tag>)> {
        self.items
            .iter()
            .map(|entry| {
                let (name, tags) = entry.val;
                (name.clone(), tags.read().val.into_iter().collect())
            })
            .collect()
    }

    pub fn contains(&self, name: &ItemName) -> bool {
        self.items.get(name).val.is_some()
    }

    pub fn tags(&self, name: &ItemName) -> BTreeSet<Tag> {
        self.items
            .get(name)
            .val
            .map(|tags| tags.read().val.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn edit(&mut self, by: DeviceId, edit: Edit) {
        let op = match edit {
            Edit::AddItem { name, tags } => {
                let ctx = self.items.read_ctx().derive_add_ctx(by);
                // An empty `add_all` still registers the entry, so a tagless item exists.
                self.items.update(name, ctx, |tags_crdt, ctx| tags_crdt.add_all(tags, ctx))
            }
            Edit::RemoveItem { name } => {
                let ctx = self.items.get(&name).derive_rm_ctx();
                self.items.rm(name, ctx)
            }
            Edit::AddTag { name, tag } => {
                let ctx = self.items.read_ctx().derive_add_ctx(by);
                self.items.update(name, ctx, |tags, ctx| tags.add(tag, ctx))
            }
            Edit::RemoveTag { name, tag } => {
                let Some(tags) = self.items.get(&name).val else {
                    return;
                };
                let rm_ctx = tags.contains(&tag).derive_rm_ctx();
                let ctx = self.items.read_ctx().derive_add_ctx(by);
                // Through `update`, not `rm`, so the item itself stays present.
                self.items.update(name, ctx, |tags, _| tags.rm(tag, rm_ctx))
            }
        };
        self.items.apply(op);
    }

    pub fn merge(&mut self, other: Self) {
        self.items.merge(other.items);
    }
}

#[derive(Debug)]
pub struct Session {
    device: DeviceId,
    list: ShoppingList,
    history: Vec<Vec<Edit>>,
}

impl Session {
    pub fn new(device: DeviceId, list: ShoppingList) -> Self {
        Self { device, list, history: Vec::new() }
    }

    pub fn list(&self) -> &ShoppingList {
        &self.list
    }

    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn edit(&mut self, edit: Edit) {
        let revert = self.revert_of(&edit);
        self.history.push(revert);
        self.list.edit(self.device, edit);
    }

    pub fn undo(&mut self) {
        for edit in self.history.pop().unwrap_or_default() {
            self.list.edit(self.device, edit);
        }
    }

    pub fn merge(&mut self, remote: ShoppingList) {
        self.list.merge(remote);
    }

    fn revert_of(&self, edit: &Edit) -> Vec<Edit> {
        match edit {
            Edit::AddItem { name, tags } if self.list.contains(name) => {
                let known = self.list.tags(name);
                tags.difference(&known)
                    .map(|tag| Edit::RemoveTag { name: name.clone(), tag: tag.clone() })
                    .collect()
            }
            Edit::AddItem { name, .. } => vec![Edit::RemoveItem { name: name.clone() }],
            Edit::RemoveItem { name } if self.list.contains(name) => {
                vec![Edit::AddItem { name: name.clone(), tags: self.list.tags(name) }]
            }
            Edit::AddTag { name, tag } if !self.list.tags(name).contains(tag) => {
                vec![Edit::RemoveTag { name: name.clone(), tag: tag.clone() }]
            }
            Edit::RemoveTag { name, tag } if self.list.tags(name).contains(tag) => {
                vec![Edit::AddTag { name: name.clone(), tag: tag.clone() }]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: DeviceId = DeviceId::new(1);

    fn name(raw: &str) -> ItemName {
        ItemName::new(raw).unwrap()
    }

    fn tag(raw: &str) -> Tag {
        Tag::new(raw).unwrap()
    }

    #[test]
    fn removing_the_last_tag_keeps_the_item() {
        let mut list = ShoppingList::default();
        list.edit(ALICE, Edit::add_item(name("Milch")));
        list.edit(ALICE, Edit::AddTag { name: name("Milch"), tag: tag("2x") });
        list.edit(ALICE, Edit::RemoveTag { name: name("Milch"), tag: tag("2x") });

        assert_eq!(list.items(), vec![(name("Milch"), BTreeSet::new())]);
    }

    #[test]
    fn undo_restores_a_ticked_off_item_with_its_tags() {
        let mut session = Session::new(ALICE, ShoppingList::default());
        session.edit(Edit::add_item(name("Milch")));
        session.edit(Edit::AddTag { name: name("Milch"), tag: tag("2x") });
        session.edit(Edit::RemoveItem { name: name("Milch") });

        session.undo();

        assert_eq!(session.list().tags(&name("Milch")), BTreeSet::from([tag("2x")]));
    }

    #[test]
    fn undo_of_adding_an_item_that_was_already_there_keeps_it() {
        let mut session = Session::new(ALICE, ShoppingList::default());
        session.edit(Edit::add_item(name("Milch")));
        session.edit(Edit::add_item(name("Milch")));

        session.undo();

        assert!(session.list().contains(&name("Milch")));
    }
}
