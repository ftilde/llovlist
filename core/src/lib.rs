use std::collections::{BTreeSet, HashMap};
use std::fmt;

use crdts::ctx::AddCtx;
use crdts::identifier::Identifier;
use crdts::{CmRDT, CvRDT, MVReg, Map, Orswot, ResetRemove, VClock, map, mvreg};
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
pub struct TrimStr(String);

impl TrimStr {
    pub fn new(raw: &str) -> Result<Self, Blank> {
        trimmed(raw).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for TrimStr {
    type Error = Blank;

    fn try_from(raw: String) -> Result<Self, Blank> {
        Self::new(&raw)
    }
}

impl From<TrimStr> for String {
    fn from(value: TrimStr) -> Self {
        value.0
    }
}

impl fmt::Display for TrimStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Sort position, dense so that a move writes one key and never renumbers its neighbours.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OrderKey(Identifier<DeviceId>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    // Keys are computed by the caller, not derived on apply, so replaying an edit during
    // undo cannot land something somewhere the user never chose.
    CreateList {
        list: TrimStr,
        key: OrderKey,
    },
    MoveList {
        list: TrimStr,
        key: OrderKey,
    },
    RemoveList {
        list: TrimStr,
    },
    AddItem {
        list: TrimStr,
        name: TrimStr,
        tags: BTreeSet<TrimStr>,
        key: OrderKey,
    },
    MoveItem {
        list: TrimStr,
        name: TrimStr,
        key: OrderKey,
    },
    RemoveItem {
        list: TrimStr,
        name: TrimStr,
    },
    AddTag {
        list: TrimStr,
        name: TrimStr,
        tag: TrimStr,
    },
    RemoveTag {
        list: TrimStr,
        name: TrimStr,
        tag: TrimStr,
    },
}

impl Edit {
    pub fn add_item(list: TrimStr, name: TrimStr, key: OrderKey) -> Self {
        Self::AddItem {
            list,
            name,
            tags: BTreeSet::new(),
            key,
        }
    }
}

type Tags = Orswot<TrimStr, DeviceId>;
type Items = Map<TrimStr, Placed<Tags>, DeviceId>;
type ListsOp = map::Op<TrimStr, Placed<Items>, DeviceId>;

/// A CRDT and its sort position as one map value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed<V> {
    position: MVReg<OrderKey, DeviceId>,
    inner: V,
}

impl<V> Placed<V> {
    fn key(&self) -> Option<OrderKey> {
        // Lowest of the concurrent writes, so every replica resolves a move the same way.
        self.position.read().val.into_iter().min()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedOp<Op> {
    position: Option<mvreg::Op<OrderKey, DeviceId>>,
    inner: Option<Op>,
}

impl<Op> PlacedOp<Op> {
    fn inner(op: Op) -> Self {
        Self {
            position: None,
            inner: Some(op),
        }
    }

    fn place(key: OrderKey, ctx: &AddCtx<DeviceId>) -> Self {
        Self {
            position: Some(put(key, ctx)),
            inner: None,
        }
    }
}

fn put(key: OrderKey, ctx: &AddCtx<DeviceId>) -> mvreg::Op<OrderKey, DeviceId> {
    mvreg::Op::Put {
        clock: ctx.clock.clone(),
        val: key,
    }
}

impl<V: CmRDT> CmRDT for Placed<V> {
    type Op = PlacedOp<V::Op>;
    type Validation = V::Validation;

    fn validate_op(&self, op: &Self::Op) -> Result<(), Self::Validation> {
        op.inner
            .as_ref()
            .map_or(Ok(()), |op| self.inner.validate_op(op))
    }

    fn apply(&mut self, op: Self::Op) {
        if let Some(op) = op.position {
            self.position.apply(op);
        }
        if let Some(op) = op.inner {
            self.inner.apply(op);
        }
    }
}

impl<V: CvRDT> CvRDT for Placed<V> {
    type Validation = V::Validation;

    fn validate_merge(&self, other: &Self) -> Result<(), Self::Validation> {
        self.inner.validate_merge(&other.inner)
    }

    fn merge(&mut self, other: Self) {
        self.position.merge(other.position);
        self.inner.merge(other.inner);
    }
}

impl<V: ResetRemove<DeviceId>> ResetRemove<DeviceId> for Placed<V> {
    fn reset_remove(&mut self, clock: &VClock<DeviceId>) {
        self.position.reset_remove(clock);
        self.inner.reset_remove(clock);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lists {
    lists: Map<TrimStr, Placed<Items>, DeviceId>,
}

/// Sorts by position, unpositioned entries last, ties by name.
fn sort_key(key: Option<OrderKey>, name: &TrimStr) -> (bool, Option<OrderKey>, TrimStr) {
    (key.is_none(), key, name.clone())
}

impl Lists {
    pub fn names(&self) -> Vec<TrimStr> {
        let mut names: Vec<TrimStr> = self.lists.keys().map(|entry| entry.val.clone()).collect();
        names.sort_by_cached_key(|name| sort_key(self.order_key(name), name));
        names
    }

    pub fn items(&self, list: &TrimStr) -> Vec<(TrimStr, BTreeSet<TrimStr>)> {
        let Some(list) = self.list(list) else {
            return Vec::new();
        };
        let mut items: Vec<(Option<OrderKey>, TrimStr, BTreeSet<TrimStr>)> = list
            .inner
            .iter()
            .map(|entry| {
                let (name, item) = entry.val;
                let tags = item.inner.read().val.into_iter().collect();
                (item.key(), name.clone(), tags)
            })
            .collect();
        items.sort_by_cached_key(|(key, name, _)| sort_key(key.clone(), name));
        items
            .into_iter()
            .map(|(_, name, tags)| (name, tags))
            .collect()
    }

    fn list(&self, list: &TrimStr) -> Option<Placed<Items>> {
        self.lists.get(list).val
    }

    fn item(&self, list: &TrimStr, name: &TrimStr) -> Option<Placed<Tags>> {
        self.list(list)?.inner.get(name).val
    }

    pub fn order_key(&self, list: &TrimStr) -> Option<OrderKey> {
        self.list(list)?.key()
    }

    pub fn item_key(&self, list: &TrimStr, name: &TrimStr) -> Option<OrderKey> {
        self.item(list, name)?.key()
    }

    pub fn key_between(
        &self,
        by: DeviceId,
        after: Option<&TrimStr>,
        before: Option<&TrimStr>,
    ) -> OrderKey {
        let lower = after.and_then(|list| self.order_key(list));
        let upper = before.and_then(|list| self.order_key(list));
        Self::between(by, lower, upper)
    }

    pub fn item_key_between(
        &self,
        by: DeviceId,
        list: &TrimStr,
        after: Option<&TrimStr>,
        before: Option<&TrimStr>,
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

    pub fn item_key_at_end(&self, by: DeviceId, list: &TrimStr) -> OrderKey {
        let items = self.items(list);
        self.item_key_between(by, list, items.last().map(|(name, _)| name), None)
    }

    /// Position putting `list` at `slot` of the order with `list` taken out.
    pub fn key_for_slot(&self, by: DeviceId, list: &TrimStr, slot: usize) -> Option<OrderKey> {
        let (after, before) = neighbours_at(self.names(), list, slot)?;
        Some(self.key_between(by, after.as_ref(), before.as_ref()))
    }

    pub fn item_key_for_slot(
        &self,
        by: DeviceId,
        list: &TrimStr,
        name: &TrimStr,
        slot: usize,
    ) -> Option<OrderKey> {
        let (after, before) = neighbours_at(self.item_names(list), name, slot)?;
        Some(self.item_key_between(by, list, after.as_ref(), before.as_ref()))
    }

    /// Position moving `list` `offset` places, or `None` if it cannot move that far.
    pub fn key_for_move(&self, by: DeviceId, list: &TrimStr, offset: isize) -> Option<OrderKey> {
        let slot = step(&self.names(), list, offset)?;
        self.key_for_slot(by, list, slot)
    }

    pub fn item_key_for_move(
        &self,
        by: DeviceId,
        list: &TrimStr,
        name: &TrimStr,
        offset: isize,
    ) -> Option<OrderKey> {
        let slot = step(&self.item_names(list), name, offset)?;
        self.item_key_for_slot(by, list, name, slot)
    }

    pub fn item_names(&self, list: &TrimStr) -> Vec<TrimStr> {
        self.items(list).into_iter().map(|(name, _)| name).collect()
    }

    pub fn has(&self, list: &TrimStr) -> bool {
        self.list(list).is_some()
    }

    pub fn contains(&self, list: &TrimStr, name: &TrimStr) -> bool {
        self.item(list, name).is_some()
    }

    pub fn tags(&self, list: &TrimStr, name: &TrimStr) -> BTreeSet<TrimStr> {
        self.item(list, name)
            .map(|item| item.inner.read().val.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn edit(&mut self, by: DeviceId, edit: Edit) {
        let ctx = self.lists.read_ctx().derive_add_ctx(by);
        let op = match edit {
            Edit::CreateList { list, key } => {
                // Only when new, so re-creating an existing list does not move it.
                let fresh = !self.has(&list);
                self.lists.update(list, ctx, |_, ctx| PlacedOp {
                    position: fresh.then(|| put(key, &ctx)),
                    inner: None,
                })
            }
            Edit::MoveList { list, key } => {
                if !self.has(&list) {
                    return;
                }
                self.lists
                    .update(list, ctx, |_, ctx| PlacedOp::place(key, &ctx))
            }
            Edit::RemoveList { list } => {
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
                self.update_item(list, name, ctx, |item, ctx| PlacedOp {
                    position: fresh.then(|| put(key, &ctx)),
                    inner: Some(item.inner.add_all(tags, ctx)),
                })
            }
            Edit::MoveItem { list, name, key } => {
                if !self.contains(&list, &name) {
                    return;
                }
                self.update_item(list, name, ctx, |_, ctx| PlacedOp::place(key, &ctx))
            }
            Edit::RemoveItem { list, name } => {
                let Some(items) = self.list(&list) else {
                    return;
                };
                let rm_ctx = items.inner.get(&name).derive_rm_ctx();
                self.lists.update(list, ctx, |list, _| {
                    PlacedOp::inner(list.inner.rm(name, rm_ctx))
                })
            }
            Edit::AddTag { list, name, tag } => self.update_item(list, name, ctx, |item, ctx| {
                PlacedOp::inner(item.inner.add(tag, ctx))
            }),
            Edit::RemoveTag { list, name, tag } => {
                let Some(item) = self.item(&list, &name) else {
                    return;
                };
                let rm_ctx = item.inner.contains(&tag).derive_rm_ctx();
                // Through `update`, not `rm`, so the item itself stays present.
                self.update_item(list, name, ctx, |item, _| {
                    PlacedOp::inner(item.inner.rm(tag, rm_ctx))
                })
            }
        };
        self.lists.apply(op);
    }

    fn update_item(
        &self,
        list: TrimStr,
        name: TrimStr,
        ctx: AddCtx<DeviceId>,
        f: impl FnOnce(&Placed<Tags>, AddCtx<DeviceId>) -> PlacedOp<<Tags as CmRDT>::Op>,
    ) -> ListsOp {
        self.lists.update(list, ctx, |list, ctx| {
            PlacedOp::inner(list.inner.update(name, ctx, f))
        })
    }

    pub fn merge(&mut self, other: Self) {
        self.lists.merge(other.lists);
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

#[derive(Debug)]
pub struct Session {
    device: DeviceId,
    lists: Lists,
    history: HashMap<TrimStr, Vec<Vec<Edit>>>,
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

    pub fn can_undo(&self, list: &TrimStr) -> bool {
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

    pub fn item_key_at_end(&self, list: &TrimStr) -> OrderKey {
        self.lists.item_key_at_end(self.device, list)
    }

    pub fn key_for_move(&self, list: &TrimStr, offset: isize) -> Option<OrderKey> {
        self.lists.key_for_move(self.device, list, offset)
    }

    pub fn item_key_for_move(
        &self,
        list: &TrimStr,
        name: &TrimStr,
        offset: isize,
    ) -> Option<OrderKey> {
        self.lists
            .item_key_for_move(self.device, list, name, offset)
    }

    pub fn key_for_slot(&self, list: &TrimStr, slot: usize) -> Option<OrderKey> {
        self.lists.key_for_slot(self.device, list, slot)
    }

    pub fn item_key_for_slot(
        &self,
        list: &TrimStr,
        name: &TrimStr,
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

    pub fn undo(&mut self, list: &TrimStr) {
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

    fn list_key(&self, list: &TrimStr) -> OrderKey {
        self.lists
            .order_key(list)
            .unwrap_or_else(|| self.key_at_end())
    }

    fn item_key(&self, list: &TrimStr, name: &TrimStr) -> OrderKey {
        self.lists
            .item_key(list, name)
            .unwrap_or_else(|| self.item_key_at_end(list))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: DeviceId = DeviceId::new(1);

    fn list(raw: &str) -> TrimStr {
        TrimStr::new(raw).unwrap()
    }

    fn name(raw: &str) -> TrimStr {
        TrimStr::new(raw).unwrap()
    }

    fn tag(raw: &str) -> TrimStr {
        TrimStr::new(raw).unwrap()
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

    fn item_order(session: &Session) -> Vec<TrimStr> {
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
