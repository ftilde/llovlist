use std::collections::{BTreeSet, HashMap};
use std::fmt;

use crdts::identifier::Identifier;
use crdts::{CmRDT, CvRDT, MVReg, Map, Orswot, VClock, map};
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

/// Sort position, dense so that a move writes one key and never renumbers its neighbours.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OrderKey(Identifier<DeviceId>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    // Keys are computed by the caller, not derived on apply, so replaying an edit during
    // undo cannot land something somewhere the user never chose.
    CreateList {
        list: ListName,
        key: OrderKey,
    },
    MoveList {
        list: ListName,
        key: OrderKey,
    },
    RemoveList {
        list: ListName,
    },
    AddItem {
        list: ListName,
        name: ItemName,
        tags: BTreeSet<Tag>,
        key: OrderKey,
    },
    MoveItem {
        list: ListName,
        name: ItemName,
        key: OrderKey,
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
    pub fn add_item(list: ListName, name: ItemName, key: OrderKey) -> Self {
        Self::AddItem {
            list,
            name,
            tags: BTreeSet::new(),
            key,
        }
    }
}

type Items = Map<ItemName, Orswot<Tag, DeviceId>, DeviceId>;
type Positions<K> = Map<K, MVReg<OrderKey, DeviceId>, DeviceId>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lists {
    lists: Map<ListName, Items, DeviceId>,
    // Sibling maps rather than fields on each entry, so every value stays a single CRDT
    // and needs no wrapper. Removals have to clear these explicitly: reset_remove only
    // reaches registers nested inside the entry being removed.
    order: Positions<ListName>,
    item_order: Map<ListName, Positions<ItemName>, DeviceId>,
}

impl Lists {
    pub fn names(&self) -> Vec<ListName> {
        let mut names: Vec<ListName> = self.lists.keys().map(|entry| entry.val.clone()).collect();
        names.sort_by_cached_key(|name| {
            let key = self.order_key(name);
            (key.is_none(), key, name.clone())
        });
        names
    }

    pub fn items(&self, list: &ListName) -> Vec<(ItemName, BTreeSet<Tag>)> {
        let mut items: Vec<(ItemName, BTreeSet<Tag>)> = self
            .lists
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
            .unwrap_or_default();
        items.sort_by_cached_key(|(name, _)| {
            let key = self.item_key(list, name);
            (key.is_none(), key, name.clone())
        });
        items
    }

    /// Lowest of the positions written for `list`, so concurrent moves still agree.
    pub fn order_key(&self, list: &ListName) -> Option<OrderKey> {
        Self::lowest(self.order.get(list).val.as_ref())
    }

    pub fn item_key(&self, list: &ListName, name: &ItemName) -> Option<OrderKey> {
        let positions = self.item_order.get(list).val?;
        Self::lowest(positions.get(name).val.as_ref())
    }

    fn lowest(positions: Option<&MVReg<OrderKey, DeviceId>>) -> Option<OrderKey> {
        positions?.read().val.into_iter().min()
    }

    pub fn key_between(
        &self,
        by: DeviceId,
        after: Option<&ListName>,
        before: Option<&ListName>,
    ) -> OrderKey {
        let lower = after.and_then(|list| self.order_key(list));
        let upper = before.and_then(|list| self.order_key(list));
        Self::between(by, lower, upper)
    }

    pub fn item_key_between(
        &self,
        by: DeviceId,
        list: &ListName,
        after: Option<&ItemName>,
        before: Option<&ItemName>,
    ) -> OrderKey {
        let lower = after.and_then(|name| self.item_key(list, name));
        let upper = before.and_then(|name| self.item_key(list, name));
        Self::between(by, lower, upper)
    }

    fn between(by: DeviceId, lower: Option<OrderKey>, upper: Option<OrderKey>) -> OrderKey {
        OrderKey(Identifier::between(
            lower.as_ref().map(|key| &key.0),
            upper.as_ref().map(|key| &key.0),
            by,
        ))
    }

    pub fn key_at_end(&self, by: DeviceId) -> OrderKey {
        self.key_between(by, self.names().last(), None)
    }

    pub fn item_key_at_end(&self, by: DeviceId, list: &ListName) -> OrderKey {
        let items = self.items(list);
        self.item_key_between(by, list, items.last().map(|(name, _)| name), None)
    }

    /// Position putting `list` at `slot` of the order with `list` taken out.
    pub fn key_for_slot(&self, by: DeviceId, list: &ListName, slot: usize) -> Option<OrderKey> {
        let (after, before) = neighbours_at(self.names(), list, slot)?;
        Some(self.key_between(by, after.as_ref(), before.as_ref()))
    }

    pub fn item_key_for_slot(
        &self,
        by: DeviceId,
        list: &ListName,
        name: &ItemName,
        slot: usize,
    ) -> Option<OrderKey> {
        let (after, before) = neighbours_at(self.item_names(list), name, slot)?;
        Some(self.item_key_between(by, list, after.as_ref(), before.as_ref()))
    }

    /// Position moving `list` `offset` places, or `None` if it cannot move that far.
    pub fn key_for_move(&self, by: DeviceId, list: &ListName, offset: isize) -> Option<OrderKey> {
        let slot = step(&self.names(), list, offset)?;
        self.key_for_slot(by, list, slot)
    }

    pub fn item_key_for_move(
        &self,
        by: DeviceId,
        list: &ListName,
        name: &ItemName,
        offset: isize,
    ) -> Option<OrderKey> {
        let slot = step(&self.item_names(list), name, offset)?;
        self.item_key_for_slot(by, list, name, slot)
    }

    pub fn item_names(&self, list: &ListName) -> Vec<ItemName> {
        self.items(list).into_iter().map(|(name, _)| name).collect()
    }

    pub fn has(&self, list: &ListName) -> bool {
        self.lists.get(list).val.is_some()
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
            Edit::CreateList { list, key } => {
                let fresh = !self.has(&list);
                let ctx = self.lists.read_ctx().derive_add_ctx(by);
                let op = self.lists.update(list.clone(), ctx, |_, _| inert());
                // Only when new, so re-creating an existing list does not move it.
                if fresh {
                    self.write_list_key(by, list, key);
                }
                op
            }
            Edit::MoveList { list, key } => {
                if self.has(&list) {
                    self.write_list_key(by, list, key);
                }
                return;
            }
            Edit::RemoveList { list } => {
                let order_ctx = self.order.get(&list).derive_rm_ctx();
                let order_op = self.order.rm(list.clone(), order_ctx);
                self.order.apply(order_op);
                let items_ctx = self.item_order.get(&list).derive_rm_ctx();
                let items_op = self.item_order.rm(list.clone(), items_ctx);
                self.item_order.apply(items_op);
                let ctx = self.lists.get(&list).derive_rm_ctx();
                self.lists.rm(list, ctx)
            }
            Edit::AddItem {
                list,
                name,
                tags,
                key,
            } => {
                let fresh = !self.contains(&list, &name);
                let ctx = self.lists.read_ctx().derive_add_ctx(by);
                let op = self.lists.update(list.clone(), ctx, |items, ctx| {
                    // An empty `add_all` still registers the entry, so a tagless item exists.
                    items.update(name.clone(), ctx, |item, ctx| item.add_all(tags, ctx))
                });
                if fresh {
                    self.write_item_key(by, list, name, key);
                }
                op
            }
            Edit::MoveItem { list, name, key } => {
                if self.contains(&list, &name) {
                    self.write_item_key(by, list, name, key);
                }
                return;
            }
            Edit::RemoveItem { list, name } => {
                let Some(items) = self.lists.get(&list).val else {
                    return;
                };
                let rm_ctx = items.get(&name).derive_rm_ctx();
                let ctx = self.lists.read_ctx().derive_add_ctx(by);
                let op = self
                    .lists
                    .update(list.clone(), ctx, |items, _| items.rm(name.clone(), rm_ctx));
                self.clear_item_key(by, list, name);
                op
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

    fn write_list_key(&mut self, by: DeviceId, list: ListName, key: OrderKey) {
        let ctx = self.order.read_ctx().derive_add_ctx(by);
        let op = self
            .order
            .update(list, ctx, |positions, ctx| positions.write(key, ctx));
        self.order.apply(op);
    }

    fn write_item_key(&mut self, by: DeviceId, list: ListName, name: ItemName, key: OrderKey) {
        let ctx = self.item_order.read_ctx().derive_add_ctx(by);
        let op = self.item_order.update(list, ctx, |positions, ctx| {
            positions.update(name, ctx, |position, ctx| position.write(key, ctx))
        });
        self.item_order.apply(op);
    }

    fn clear_item_key(&mut self, by: DeviceId, list: ListName, name: ItemName) {
        let Some(positions) = self.item_order.get(&list).val else {
            return;
        };
        let rm_ctx = positions.get(&name).derive_rm_ctx();
        let ctx = self.item_order.read_ctx().derive_add_ctx(by);
        let op = self
            .item_order
            .update(list, ctx, |positions, _| positions.rm(name, rm_ctx));
        self.item_order.apply(op);
    }

    pub fn merge(&mut self, other: Self) {
        self.lists.merge(other.lists);
        self.order.merge(other.order);
        self.item_order.merge(other.item_order);
    }
}

/// The entries `subject` would sit between at `slot` of the order with it taken out.
fn neighbours_at<T: PartialEq + Clone>(
    order: Vec<T>,
    subject: &T,
    slot: usize,
) -> Option<(Option<T>, Option<T>)> {
    let at = order.iter().position(|entry| entry == subject)?;
    let mut without = order;
    without.remove(at);
    // One past the end is a valid landing place; further is not.
    if slot > without.len() {
        return None;
    }
    Some((
        slot.checked_sub(1)
            .and_then(|index| without.get(index))
            .cloned(),
        without.get(slot).cloned(),
    ))
}

/// Slot `offset` places from where `subject` sits, or `None` if that leaves the order.
fn step<T: PartialEq>(order: &[T], subject: &T, offset: isize) -> Option<usize> {
    let at = order.iter().position(|entry| entry == subject)?;
    let slot = usize::try_from(at as isize + offset).ok()?;
    (slot < order.len()).then_some(slot)
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
    history: HashMap<ListName, Vec<Vec<Edit>>>,
    deleted: Vec<Vec<Edit>>,
}

impl Session {
    pub fn new(device: DeviceId, lists: Lists) -> Self {
        Self {
            device,
            lists,
            history: HashMap::new(),
            deleted: Vec::new(),
        }
    }

    pub fn lists(&self) -> &Lists {
        &self.lists
    }

    pub fn can_undo(&self, list: &ListName) -> bool {
        self.history
            .get(list)
            .is_some_and(|stack| !stack.is_empty())
    }

    pub fn can_restore(&self) -> bool {
        !self.deleted.is_empty()
    }

    pub fn key_at_end(&self) -> OrderKey {
        self.lists.key_at_end(self.device)
    }

    pub fn item_key_at_end(&self, list: &ListName) -> OrderKey {
        self.lists.item_key_at_end(self.device, list)
    }

    pub fn key_for_move(&self, list: &ListName, offset: isize) -> Option<OrderKey> {
        self.lists.key_for_move(self.device, list, offset)
    }

    pub fn item_key_for_move(
        &self,
        list: &ListName,
        name: &ItemName,
        offset: isize,
    ) -> Option<OrderKey> {
        self.lists
            .item_key_for_move(self.device, list, name, offset)
    }

    pub fn key_for_slot(&self, list: &ListName, slot: usize) -> Option<OrderKey> {
        self.lists.key_for_slot(self.device, list, slot)
    }

    pub fn item_key_for_slot(
        &self,
        list: &ListName,
        name: &ItemName,
        slot: usize,
    ) -> Option<OrderKey> {
        self.lists.item_key_for_slot(self.device, list, name, slot)
    }

    pub fn edit(&mut self, edit: Edit) {
        let revert = self.revert_of(&edit);
        match &edit {
            // Deleting a list outlives the list, so its restore goes on its own stack --
            // and the list's own history describes a state that no longer exists.
            Edit::RemoveList { list } => {
                self.history.remove(list);
                if !revert.is_empty() {
                    self.deleted.push(revert);
                }
            }
            // Reordering is cheap to redo by hand and undoing it surprises more than it
            // helps, so it records nothing.
            Edit::CreateList { .. } | Edit::MoveList { .. } | Edit::MoveItem { .. } => {}
            Edit::AddItem { list, .. }
            | Edit::RemoveItem { list, .. }
            | Edit::AddTag { list, .. }
            | Edit::RemoveTag { list, .. } => {
                if !revert.is_empty() {
                    self.history.entry(list.clone()).or_default().push(revert);
                }
            }
        }
        self.lists.edit(self.device, edit);
    }

    pub fn undo(&mut self, list: &ListName) {
        let Some(revert) = self.history.get_mut(list).and_then(Vec::pop) else {
            return;
        };
        for edit in revert {
            self.lists.edit(self.device, edit);
        }
    }

    /// Puts the most recently deleted list back, with its items, tags and position.
    pub fn restore_deleted(&mut self) {
        for edit in self.deleted.pop().unwrap_or_default() {
            self.lists.edit(self.device, edit);
        }
    }

    pub fn merge(&mut self, remote: Lists) {
        self.lists.merge(remote);
    }

    fn revert_of(&self, edit: &Edit) -> Vec<Edit> {
        match edit {
            Edit::CreateList { .. } | Edit::MoveList { .. } | Edit::MoveItem { .. } => Vec::new(),
            Edit::RemoveList { list } if self.lists.has(list) => {
                let mut restore = vec![Edit::CreateList {
                    list: list.clone(),
                    key: self.list_key(list),
                }];
                restore.extend(self.lists.items(list).into_iter().map(|(name, tags)| {
                    Edit::AddItem {
                        key: self.item_key(list, &name),
                        list: list.clone(),
                        name,
                        tags,
                    }
                }));
                restore
            }
            Edit::AddItem {
                list, name, tags, ..
            } if self.lists.contains(list, name) => {
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
                    key: self.item_key(list, name),
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

    fn list_key(&self, list: &ListName) -> OrderKey {
        self.lists
            .order_key(list)
            .unwrap_or_else(|| self.key_at_end())
    }

    fn item_key(&self, list: &ListName, name: &ItemName) -> OrderKey {
        self.lists
            .item_key(list, name)
            .unwrap_or_else(|| self.item_key_at_end(list))
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

    fn create(session: &Session, name: &str) -> Edit {
        Edit::CreateList {
            key: session.key_at_end(),
            list: list(name),
        }
    }

    fn add(session: &Session, in_list: &str, item: &str) -> Edit {
        Edit::add_item(
            list(in_list),
            name(item),
            session.item_key_at_end(&list(in_list)),
        )
    }

    fn three_lists() -> Session {
        let mut session = Session::new(ALICE, Lists::default());
        for entry in ["a", "b", "c"] {
            let edit = create(&session, entry);
            session.edit(edit);
        }
        session
    }

    fn home_with_items() -> Session {
        let mut session = Session::new(ALICE, Lists::default());
        for item in ["milch", "brot", "kaffee"] {
            let edit = add(&session, "home", item);
            session.edit(edit);
        }
        session
    }

    fn item_order(session: &Session) -> Vec<ItemName> {
        session
            .lists()
            .items(&list("home"))
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    #[test]
    fn items_keep_the_order_they_were_added_in() {
        let session = home_with_items();

        assert_eq!(
            item_order(&session),
            [name("milch"), name("brot"), name("kaffee")]
        );
    }

    #[test]
    fn an_item_moves_one_place_at_a_time() {
        let mut session = home_with_items();

        let key = session
            .item_key_for_move(&list("home"), &name("kaffee"), -1)
            .unwrap();
        session.edit(Edit::MoveItem {
            list: list("home"),
            name: name("kaffee"),
            key,
        });
        assert_eq!(
            item_order(&session),
            [name("milch"), name("kaffee"), name("brot")]
        );

        let key = session
            .item_key_for_move(&list("home"), &name("kaffee"), -1)
            .unwrap();
        session.edit(Edit::MoveItem {
            list: list("home"),
            name: name("kaffee"),
            key,
        });
        assert_eq!(
            item_order(&session),
            [name("kaffee"), name("milch"), name("brot")]
        );
    }

    #[test]
    fn an_item_can_be_dropped_into_any_slot() {
        let mut session = home_with_items();

        for (slot, expected) in [
            (0, ["kaffee", "milch", "brot"]),
            (1, ["milch", "kaffee", "brot"]),
            (2, ["milch", "brot", "kaffee"]),
        ] {
            let key = session
                .item_key_for_slot(&list("home"), &name("kaffee"), slot)
                .unwrap();
            session.edit(Edit::MoveItem {
                list: list("home"),
                name: name("kaffee"),
                key,
            });
            assert_eq!(
                item_order(&session),
                expected.map(name).to_vec(),
                "dropping kaffee into slot {slot}"
            );
        }
    }

    #[test]
    fn a_slot_past_the_end_is_refused() {
        let session = home_with_items();

        // Three items, so slots 0..=2 exist once the dragged one is taken out.
        assert!(
            session
                .item_key_for_slot(&list("home"), &name("kaffee"), 2)
                .is_some()
        );
        assert_eq!(
            session.item_key_for_slot(&list("home"), &name("kaffee"), 3),
            None
        );
    }

    #[test]
    fn an_item_cannot_move_past_either_end() {
        let session = home_with_items();

        assert_eq!(
            session.item_key_for_move(&list("home"), &name("milch"), -1),
            None
        );
        assert_eq!(
            session.item_key_for_move(&list("home"), &name("kaffee"), 1),
            None
        );
        assert_eq!(
            session.item_key_for_move(&list("home"), &name("nope"), 1),
            None
        );
    }

    #[test]
    fn moving_an_item_records_nothing_to_undo() {
        let mut session = home_with_items();
        session.edit(Edit::RemoveItem {
            list: list("home"),
            name: name("brot"),
        });

        let key = session
            .item_key_for_move(&list("home"), &name("kaffee"), -1)
            .unwrap();
        session.edit(Edit::MoveItem {
            list: list("home"),
            name: name("kaffee"),
            key,
        });
        session.undo(&list("home"));

        // The move is left alone; undo reaches past it to the tick-off.
        assert_eq!(
            item_order(&session),
            [name("kaffee"), name("milch"), name("brot")]
        );
    }

    #[test]
    fn undo_of_a_tick_off_restores_the_items_place() {
        let mut session = home_with_items();

        session.edit(Edit::RemoveItem {
            list: list("home"),
            name: name("brot"),
        });
        session.undo(&list("home"));

        assert_eq!(
            item_order(&session),
            [name("milch"), name("brot"), name("kaffee")]
        );
    }

    #[test]
    fn re_adding_an_item_does_not_move_it() {
        let mut session = home_with_items();

        let edit = add(&session, "home", "milch");
        session.edit(edit);

        assert_eq!(
            item_order(&session),
            [name("milch"), name("brot"), name("kaffee")]
        );
    }

    #[test]
    fn removing_the_last_tag_keeps_the_item() {
        let mut session = Session::new(ALICE, Lists::default());
        let edit = add(&session, "home", "milch");
        session.edit(edit);
        session.edit(Edit::AddTag {
            list: list("home"),
            name: name("milch"),
            tag: tag("2x"),
        });
        session.edit(Edit::RemoveTag {
            list: list("home"),
            name: name("milch"),
            tag: tag("2x"),
        });

        assert_eq!(
            session.lists().items(&list("home")),
            vec![(name("milch"), BTreeSet::new())]
        );
    }

    #[test]
    fn a_list_moves_one_place_at_a_time() {
        let mut session = three_lists();

        for (subject, offset, expected) in [
            ("c", -1, ["a", "c", "b"]),
            ("c", -1, ["c", "a", "b"]),
            ("c", 1, ["a", "c", "b"]),
            ("c", 1, ["a", "b", "c"]),
        ] {
            let key = session.key_for_move(&list(subject), offset).unwrap();
            session.edit(Edit::MoveList {
                list: list(subject),
                key,
            });
            assert_eq!(
                session.lists().names(),
                expected.map(list).to_vec(),
                "after moving {subject} by {offset}"
            );
        }
    }

    #[test]
    fn a_list_cannot_move_past_either_end() {
        let session = three_lists();

        assert_eq!(session.key_for_move(&list("a"), -1), None);
        assert_eq!(session.key_for_move(&list("c"), 1), None);
        assert_eq!(session.key_for_move(&list("nope"), 1), None);
    }

    #[test]
    fn moving_a_list_records_nothing_to_undo() {
        let mut session = three_lists();

        let key = session.key_for_move(&list("c"), -1).unwrap();
        session.edit(Edit::MoveList {
            list: list("c"),
            key,
        });

        assert!(!session.can_undo(&list("c")));
        assert!(!session.can_restore());
    }

    #[test]
    fn restoring_a_deleted_list_puts_it_back_in_place() {
        let mut session = three_lists();

        session.edit(Edit::RemoveList { list: list("b") });
        assert!(session.can_restore());
        session.restore_deleted();

        assert_eq!(
            session.lists().names(),
            vec![list("a"), list("b"), list("c")]
        );
    }

    #[test]
    fn undo_restores_a_removed_list_with_its_items_and_tags() {
        let mut session = Session::new(ALICE, Lists::default());
        let edit = add(&session, "home", "milch");
        session.edit(edit);
        session.edit(Edit::AddTag {
            list: list("home"),
            name: name("milch"),
            tag: tag("2x"),
        });
        session.edit(Edit::RemoveList { list: list("home") });

        session.restore_deleted();

        assert_eq!(session.lists().names(), vec![list("home")]);
        assert_eq!(
            session.lists().tags(&list("home"), &name("milch")),
            BTreeSet::from([tag("2x")])
        );
    }

    // An edit that changed nothing costs no undo press; the next one reaches past it.
    #[test]
    fn re_adding_an_item_records_no_undo_step() {
        let mut session = Session::new(ALICE, Lists::default());
        let first = add(&session, "home", "milch");
        session.edit(first);
        let again = add(&session, "home", "milch");
        session.edit(again);

        session.undo(&list("home"));

        assert!(!session.lists().contains(&list("home"), &name("milch")));
        assert!(!session.can_undo(&list("home")));
    }

    #[test]
    fn each_list_undoes_on_its_own() {
        let mut session = Session::new(ALICE, Lists::default());
        for (in_list, item) in [("home", "milch"), ("baumarkt", "schrauben")] {
            let edit = add(&session, in_list, item);
            session.edit(edit);
        }

        session.undo(&list("home"));

        assert!(!session.lists().contains(&list("home"), &name("milch")));
        assert!(
            session
                .lists()
                .contains(&list("baumarkt"), &name("schrauben"))
        );
        assert!(!session.can_undo(&list("home")));
        assert!(session.can_undo(&list("baumarkt")));
    }

    #[test]
    fn deleting_a_list_drops_its_undo_history() {
        let mut session = home_with_items();
        session.edit(Edit::RemoveItem {
            list: list("home"),
            name: name("brot"),
        });

        session.edit(Edit::RemoveList { list: list("home") });
        session.restore_deleted();

        // The restored list is whole, and its earlier steps are not replayable onto it.
        assert!(!session.can_undo(&list("home")));
        assert_eq!(item_order(&session), [name("milch"), name("kaffee")]);
    }
}
