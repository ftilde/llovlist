use std::collections::BTreeSet;
use std::fmt;

use crdts::{CmRDT, CvRDT, Map, Orswot, VClock, map};
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
    if trimmed.is_empty() {
        Err(Blank)
    } else {
        Ok(trimmed.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ListName(String);

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

trimmed_newtype!(ListName);
trimmed_newtype!(ItemName);
trimmed_newtype!(Tag);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    CreateList {
        list: ListName,
    },
    RemoveList {
        list: ListName,
    },
    AddItem {
        list: ListName,
        name: ItemName,
        tags: BTreeSet<Tag>,
    },
    RemoveItem {
        list: ListName,
        name: ItemName,
    },
    AddTag {
        list: ListName,
        name: ItemName,
        tag: Tag,
    },
    RemoveTag {
        list: ListName,
        name: ItemName,
        tag: Tag,
    },
}

impl Edit {
    pub fn add_item(list: ListName, name: ItemName) -> Self {
        Self::AddItem {
            list,
            name,
            tags: BTreeSet::new(),
        }
    }
}

type Items = Map<ItemName, Orswot<Tag, DeviceId>, DeviceId>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lists {
    lists: Map<ListName, Items, DeviceId>,
}

impl Lists {
    pub fn names(&self) -> Vec<ListName> {
        self.lists.keys().map(|entry| entry.val.clone()).collect()
    }

    pub fn has(&self, list: &ListName) -> bool {
        self.lists.get(list).val.is_some()
    }

    pub fn items(&self, list: &ListName) -> Vec<(ItemName, BTreeSet<Tag>)> {
        self.lists
            .get(list)
            .val
            .map(|items| {
                items
                    .iter()
                    .map(|entry| {
                        let (name, tags) = entry.val;
                        (name.clone(), tags.read().val.into_iter().collect())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn contains(&self, list: &ListName, name: &ItemName) -> bool {
        self.lists
            .get(list)
            .val
            .is_some_and(|items| items.get(name).val.is_some())
    }

    pub fn tags(&self, list: &ListName, name: &ItemName) -> BTreeSet<Tag> {
        self.lists
            .get(list)
            .val
            .and_then(|items| items.get(name).val)
            .map(|tags| tags.read().val.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn edit(&mut self, by: DeviceId, edit: Edit) {
        let op = match edit {
            Edit::CreateList { list } => {
                let ctx = self.lists.read_ctx().derive_add_ctx(by);
                self.lists.update(list, ctx, |_, _| inert())
            }
            Edit::RemoveList { list } => {
                let ctx = self.lists.get(&list).derive_rm_ctx();
                self.lists.rm(list, ctx)
            }
            Edit::AddItem { list, name, tags } => {
                let ctx = self.lists.read_ctx().derive_add_ctx(by);
                self.lists.update(list, ctx, |items, ctx| {
                    // An empty `add_all` still registers the entry, so a tagless item exists.
                    items.update(name, ctx, |item, ctx| item.add_all(tags, ctx))
                })
            }
            Edit::RemoveItem { list, name } => {
                let Some(items) = self.lists.get(&list).val else {
                    return;
                };
                let rm_ctx = items.get(&name).derive_rm_ctx();
                let ctx = self.lists.read_ctx().derive_add_ctx(by);
                self.lists
                    .update(list, ctx, |items, _| items.rm(name, rm_ctx))
            }
            Edit::AddTag { list, name, tag } => {
                let ctx = self.lists.read_ctx().derive_add_ctx(by);
                self.lists.update(list, ctx, |items, ctx| {
                    items.update(name, ctx, |tags, ctx| tags.add(tag, ctx))
                })
            }
            Edit::RemoveTag { list, name, tag } => {
                let Some(tags) = self
                    .lists
                    .get(&list)
                    .val
                    .and_then(|items| items.get(&name).val)
                else {
                    return;
                };
                let rm_ctx = tags.contains(&tag).derive_rm_ctx();
                let ctx = self.lists.read_ctx().derive_add_ctx(by);
                self.lists.update(list, ctx, |items, ctx| {
                    // Through `update`, not `rm`, so the item itself stays present.
                    items.update(name, ctx, |tags, _| tags.rm(tag, rm_ctx))
                })
            }
        };
        self.lists.apply(op);
    }

    pub fn merge(&mut self, other: Self) {
        self.lists.merge(other.lists);
    }
}

// Registering a list needs an op for its contents; the outer Up carries the dot that
// makes the entry exist, so this one only has to be harmless.
fn inert() -> map::Op<ItemName, Orswot<Tag, DeviceId>, DeviceId> {
    map::Op::Rm {
        clock: VClock::new(),
        keyset: BTreeSet::new(),
    }
}

#[derive(Debug)]
pub struct Session {
    device: DeviceId,
    lists: Lists,
    history: Vec<Vec<Edit>>,
}

impl Session {
    pub fn new(device: DeviceId, lists: Lists) -> Self {
        Self {
            device,
            lists,
            history: Vec::new(),
        }
    }

    pub fn lists(&self) -> &Lists {
        &self.lists
    }

    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn edit(&mut self, edit: Edit) {
        let revert = self.revert_of(&edit);
        self.history.push(revert);
        self.lists.edit(self.device, edit);
    }

    pub fn undo(&mut self) {
        for edit in self.history.pop().unwrap_or_default() {
            self.lists.edit(self.device, edit);
        }
    }

    pub fn merge(&mut self, remote: Lists) {
        self.lists.merge(remote);
    }

    fn revert_of(&self, edit: &Edit) -> Vec<Edit> {
        match edit {
            Edit::CreateList { list } if self.lists.has(list) => Vec::new(),
            Edit::CreateList { list } => vec![Edit::RemoveList { list: list.clone() }],
            Edit::RemoveList { list } if self.lists.has(list) => {
                let mut restore = vec![Edit::CreateList { list: list.clone() }];
                restore.extend(self.lists.items(list).into_iter().map(|(name, tags)| {
                    Edit::AddItem {
                        list: list.clone(),
                        name,
                        tags,
                    }
                }));
                restore
            }
            Edit::AddItem { list, name, tags } if self.lists.contains(list, name) => {
                let known = self.lists.tags(list, name);
                tags.difference(&known)
                    .map(|tag| Edit::RemoveTag {
                        list: list.clone(),
                        name: name.clone(),
                        tag: tag.clone(),
                    })
                    .collect()
            }
            Edit::AddItem { list, name, .. } => vec![Edit::RemoveItem {
                list: list.clone(),
                name: name.clone(),
            }],
            Edit::RemoveItem { list, name } if self.lists.contains(list, name) => {
                vec![Edit::AddItem {
                    list: list.clone(),
                    name: name.clone(),
                    tags: self.lists.tags(list, name),
                }]
            }
            Edit::AddTag { list, name, tag } if !self.lists.tags(list, name).contains(tag) => {
                vec![Edit::RemoveTag {
                    list: list.clone(),
                    name: name.clone(),
                    tag: tag.clone(),
                }]
            }
            Edit::RemoveTag { list, name, tag } if self.lists.tags(list, name).contains(tag) => {
                vec![Edit::AddTag {
                    list: list.clone(),
                    name: name.clone(),
                    tag: tag.clone(),
                }]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: DeviceId = DeviceId::new(1);

    fn list(raw: &str) -> ListName {
        ListName::new(raw).unwrap()
    }

    fn name(raw: &str) -> ItemName {
        ItemName::new(raw).unwrap()
    }

    fn tag(raw: &str) -> Tag {
        Tag::new(raw).unwrap()
    }

    #[test]
    fn removing_the_last_tag_keeps_the_item() {
        let mut lists = Lists::default();
        lists.edit(ALICE, Edit::add_item(list("home"), name("Milch")));
        lists.edit(
            ALICE,
            Edit::AddTag {
                list: list("home"),
                name: name("Milch"),
                tag: tag("2x"),
            },
        );
        lists.edit(
            ALICE,
            Edit::RemoveTag {
                list: list("home"),
                name: name("Milch"),
                tag: tag("2x"),
            },
        );

        assert_eq!(
            lists.items(&list("home")),
            vec![(name("Milch"), BTreeSet::new())]
        );
    }

    #[test]
    fn undo_restores_a_removed_list_with_its_items_and_tags() {
        let mut session = Session::new(ALICE, Lists::default());
        session.edit(Edit::add_item(list("home"), name("Milch")));
        session.edit(Edit::AddTag {
            list: list("home"),
            name: name("Milch"),
            tag: tag("2x"),
        });
        session.edit(Edit::RemoveList { list: list("home") });

        session.undo();

        assert_eq!(session.lists().names(), vec![list("home")]);
        assert_eq!(
            session.lists().tags(&list("home"), &name("Milch")),
            BTreeSet::from([tag("2x")])
        );
    }

    #[test]
    fn undo_of_adding_an_item_that_was_already_there_keeps_it() {
        let mut session = Session::new(ALICE, Lists::default());
        session.edit(Edit::add_item(list("home"), name("Milch")));
        session.edit(Edit::add_item(list("home"), name("Milch")));

        session.undo();

        assert!(session.lists().contains(&list("home"), &name("Milch")));
    }
}
