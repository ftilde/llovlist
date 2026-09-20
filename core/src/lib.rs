use std::collections::{BTreeSet, HashMap};
use std::fmt;

use crdts::ctx::AddCtx;
use crdts::identifier::Identifier;
use crdts::{CmRDT, CvRDT, MVReg, Map, Orswot, ResetRemove, VClock, map, mvreg};
use serde::{Deserialize, Serialize};

// Two replicas sharing an id mint the same dots and silently drop each other's entries.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
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
struct OrderKey(Identifier<DeviceId>);

impl OrderKey {
    fn between(by: DeviceId, lower: Option<&Self>, upper: Option<&Self>) -> Self {
        Self(Identifier::between(
            lower.map(|key| &key.0),
            upper.map(|key| &key.0),
            by,
        ))
    }
}

/// Where an entry goes in an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Position {
    Before(TrimStr),
    End,
}

/// The key for `at` in `order`, or `None` if `at` names an entry that is not there.
fn resolve(
    order: &[TrimStr],
    key_of: impl Fn(&TrimStr) -> Option<OrderKey>,
    by: DeviceId,
    at: &Position,
) -> Option<OrderKey> {
    let (lower, upper) = match at {
        Position::End => (order.last(), None),
        Position::Before(next) => {
            let index = order.iter().position(|entry| entry == next)?;
            (index.checked_sub(1).map(|index| &order[index]), Some(next))
        }
    };
    let lower = lower.and_then(&key_of);
    let upper = upper.and_then(&key_of);
    Some(OrderKey::between(by, lower.as_ref(), upper.as_ref()))
}

fn without(mut order: Vec<TrimStr>, subject: &TrimStr) -> Vec<TrimStr> {
    order.retain(|entry| entry != subject);
    order
}

/// The position landing at `slot` of `order`, one past the end included.
fn at_slot(order: &[TrimStr], slot: usize) -> Option<Position> {
    match order.get(slot) {
        Some(next) => Some(Position::Before(next.clone())),
        None if slot == order.len() => Some(Position::End),
        None => None,
    }
}

// Positions are resolved to keys when the edit is made and stored as such, so replaying
// an inverse lands where the item was, not where some name happens to be by then.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemEdit {
    AddItem {
        name: TrimStr,
        tags: BTreeSet<TrimStr>,
        key: OrderKey,
    },
    MoveItem {
        name: TrimStr,
        key: OrderKey,
    },
    RemoveItem {
        name: TrimStr,
    },
    AddTag {
        name: TrimStr,
        tag: TrimStr,
    },
    RemoveTag {
        name: TrimStr,
        tag: TrimStr,
    },
}

impl ItemEdit {
    /// The edits that take `list` back to before this one is applied.
    fn inverse(&self, by: DeviceId, list: &List) -> Option<EditGroup> {
        let edits = match self {
            // Reordering is cheap to redo by hand and undoing it surprises more than it
            // helps, so it has no inverse.
            Self::MoveItem { .. } => Vec::new(),
            Self::AddItem { name, tags, .. } if list.contains(name) => {
                let known = list.tags(name);
                tags.difference(&known)
                    .map(|tag| Self::RemoveTag {
                        name: name.clone(),
                        tag: tag.clone(),
                    })
                    .collect()
            }
            Self::AddItem { name, .. } => vec![Self::RemoveItem { name: name.clone() }],
            Self::RemoveItem { name } if list.contains(name) => vec![list.re_add(by, name)],
            Self::AddTag { name, tag } if !list.tags(name).contains(tag) => {
                vec![Self::RemoveTag {
                    name: name.clone(),
                    tag: tag.clone(),
                }]
            }
            Self::RemoveTag { name, tag } if list.tags(name).contains(tag) => {
                vec![Self::AddTag {
                    name: name.clone(),
                    tag: tag.clone(),
                }]
            }
            _ => Vec::new(),
        };
        EditGroup::collect(edits)
    }
}

/// One undo step: edits applied and undone as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditGroup(Vec<ItemEdit>);

impl EditGroup {
    fn collect(edits: Vec<ItemEdit>) -> Option<Self> {
        (!edits.is_empty()).then_some(Self(edits))
    }

    /// The edits that take `list` back to before this group is applied, if it changes anything.
    fn inverse(&self, session: &Session, lists: &Lists, list: &TrimStr) -> Option<Self> {
        // Each edit is inverted against the state the ones before it leave behind.
        let mut scratch = lists.clone();
        let mut inverses = Vec::new();
        for edit in &self.0 {
            inverses.push(edit.inverse(session.device, &scratch.list(list)));
            scratch.list_mut(session, list.clone()).edit(edit.clone());
        }
        Self::collect(
            inverses
                .into_iter()
                .rev()
                .flatten()
                .flat_map(|group| group.0)
                .collect(),
        )
    }
}

impl From<ItemEdit> for EditGroup {
    fn from(edit: ItemEdit) -> Self {
        Self(vec![edit])
    }
}

type Tags = Orswot<TrimStr, DeviceId>;
pub type Item = Placed<Tags>;
type Items = Map<TrimStr, Item, DeviceId>;
pub type List = Placed<Items>;
type ListsOp = map::Op<TrimStr, List, DeviceId>;

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

/// Sorts by position, unpositioned entries last, ties by name.
fn sort_key(key: Option<OrderKey>, name: &TrimStr) -> (bool, Option<OrderKey>, TrimStr) {
    (key.is_none(), key, name.clone())
}

impl List {
    pub fn item(&self, name: &TrimStr) -> Option<Item> {
        self.inner.get(name).val
    }

    pub fn contains(&self, name: &TrimStr) -> bool {
        self.item(name).is_some()
    }

    pub fn tags(&self, name: &TrimStr) -> BTreeSet<TrimStr> {
        self.item(name).map(|item| item.tags()).unwrap_or_default()
    }

    pub fn items(&self) -> Vec<(TrimStr, BTreeSet<TrimStr>)> {
        let mut items: Vec<(Option<OrderKey>, TrimStr, BTreeSet<TrimStr>)> = self
            .inner
            .iter()
            .map(|entry| {
                let (name, item) = entry.val;
                (item.key(), name.clone(), item.tags())
            })
            .collect();
        items.sort_by_cached_key(|(key, name, _)| sort_key(key.clone(), name));
        items
            .into_iter()
            .map(|(_, name, tags)| (name, tags))
            .collect()
    }

    pub fn item_names(&self) -> Vec<TrimStr> {
        self.items().into_iter().map(|(name, _)| name).collect()
    }

    fn item_key(&self, name: &TrimStr) -> Option<OrderKey> {
        self.item(name)?.key()
    }

    /// The key for `subject` at `at`, with `subject` taken out of the order first.
    fn key_for(&self, by: DeviceId, subject: &TrimStr, at: &Position) -> Option<OrderKey> {
        let order = without(self.item_names(), subject);
        resolve(&order, |name| self.item_key(name), by, at)
    }

    /// The edit that puts `name` back with its tags and place.
    fn re_add(&self, by: DeviceId, name: &TrimStr) -> ItemEdit {
        ItemEdit::AddItem {
            key: self.item_key(name).unwrap_or_else(|| {
                self.key_for(by, name, &Position::End)
                    .expect("End always resolves")
            }),
            name: name.clone(),
            tags: self.tags(name),
        }
    }
}

impl Item {
    pub fn tags(&self) -> BTreeSet<TrimStr> {
        self.inner.read().val.into_iter().collect()
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
    lists: Map<TrimStr, List, DeviceId>,
    // Undo state is per device and never leaves it.
    #[serde(skip)]
    history: HashMap<TrimStr, Vec<EditGroup>>,
    #[serde(skip)]
    deleted: Vec<Removed>,
}

/// What it takes to put a removed list back: its place and its items.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Removed {
    list: TrimStr,
    key: OrderKey,
    items: Vec<ItemEdit>,
}

impl Lists {
    pub fn names(&self) -> Vec<TrimStr> {
        let mut names: Vec<TrimStr> = self.lists.keys().map(|entry| entry.val.clone()).collect();
        names.sort_by_cached_key(|name| sort_key(self.order_key(name), name));
        names
    }

    pub fn has(&self, name: &TrimStr) -> bool {
        self.lists.get(name).val.is_some()
    }

    /// The list, or an empty one if there is none by that name.
    pub fn list(&self, name: &TrimStr) -> List {
        self.lists.get(name).val.unwrap_or_default()
    }

    pub fn list_mut<'a>(&'a mut self, session: &'a Session, name: TrimStr) -> ListMut<'a> {
        ListMut {
            lists: self,
            session,
            name,
        }
    }

    fn order_key(&self, name: &TrimStr) -> Option<OrderKey> {
        self.list(name).key()
    }

    /// The key for `subject` at `at`, with `subject` taken out of the order first.
    fn key_for(&self, session: &Session, subject: &TrimStr, at: &Position) -> Option<OrderKey> {
        let order = without(self.names(), subject);
        resolve(&order, |name| self.order_key(name), session.device, at)
    }

    /// Creates the list at `at`; an existing list stays where it is.
    pub fn create(&mut self, session: &Session, name: TrimStr, at: Position) {
        let Some(key) = self.key_for(session, &name, &at) else {
            return;
        };
        self.create_at(session, name, key);
    }

    fn create_at(&mut self, session: &Session, name: TrimStr, key: OrderKey) {
        // Only when new, so re-creating an existing list does not move it.
        let fresh = !self.has(&name);
        let ctx = self.lists.read_ctx().derive_add_ctx(session.device);
        let op = self.lists.update(name, ctx, |_, ctx| PlacedOp {
            position: fresh.then(|| put(key, &ctx)),
            inner: None,
        });
        self.lists.apply(op);
    }

    pub fn remove(&mut self, session: &Session, name: &TrimStr) {
        // Deleting a list outlives the list, so its restore goes on its own stack --
        // and the list's own history describes a state that no longer exists.
        self.history.remove(name);
        if self.has(name) {
            self.deleted.push(self.removed(session, name.clone()));
        }
        let ctx = self.lists.get(name).derive_rm_ctx();
        let op = self.lists.rm(name.clone(), ctx);
        self.lists.apply(op);
    }

    pub fn can_undo(&self, name: &TrimStr) -> bool {
        self.history
            .get(name)
            .is_some_and(|stack| !stack.is_empty())
    }

    pub fn can_restore(&self) -> bool {
        !self.deleted.is_empty()
    }

    /// Puts the most recently deleted list back, with its items, tags and position.
    pub fn restore_deleted(&mut self, session: &Session) {
        let Some(removed) = self.deleted.pop() else {
            return;
        };
        self.create_at(session, removed.list.clone(), removed.key);
        let mut list = self.list_mut(session, removed.list);
        for item in removed.items {
            list.edit(item);
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.lists.merge(other.lists);
    }

    fn removed(&self, session: &Session, name: TrimStr) -> Removed {
        let list = self.list(&name);
        Removed {
            key: self.order_key(&name).unwrap_or_else(|| {
                self.key_for(session, &name, &Position::End)
                    .expect("End always resolves")
            }),
            items: list
                .item_names()
                .iter()
                .map(|item| list.re_add(session.device, item))
                .collect(),
            list: name,
        }
    }
}

/// Write access to one list, recording undo steps; reads go through `Lists::list`.
pub struct ListMut<'a> {
    lists: &'a mut Lists,
    session: &'a Session,
    name: TrimStr,
}

impl ListMut<'_> {
    pub fn add_item(&mut self, name: TrimStr, at: Position) {
        let Some(key) = self.list().key_for(self.by(), &name, &at) else {
            return;
        };
        self.apply(
            ItemEdit::AddItem {
                name,
                tags: BTreeSet::new(),
                key,
            }
            .into(),
        );
    }

    pub fn move_item(&mut self, name: TrimStr, to: Position) {
        let Some(key) = self.list().key_for(self.by(), &name, &to) else {
            return;
        };
        self.apply(ItemEdit::MoveItem { name, key }.into());
    }

    /// Drops `name` into `slot` of the order with `name` taken out.
    pub fn move_item_to_slot(&mut self, name: TrimStr, slot: usize) {
        let order = without(self.list().item_names(), &name);
        if let Some(to) = at_slot(&order, slot) {
            self.move_item(name, to);
        }
    }

    pub fn remove_item(&mut self, name: TrimStr) {
        self.apply(ItemEdit::RemoveItem { name }.into());
    }

    pub fn add_tag(&mut self, name: TrimStr, tag: TrimStr) {
        self.apply(ItemEdit::AddTag { name, tag }.into());
    }

    pub fn remove_tag(&mut self, name: TrimStr, tag: TrimStr) {
        self.apply(ItemEdit::RemoveTag { name, tag }.into());
    }

    /// Moves the list itself.
    pub fn move_to(&mut self, to: Position) {
        if !self.lists.has(&self.name) {
            return;
        }
        let Some(key) = self.lists.key_for(self.session, &self.name, &to) else {
            return;
        };
        let map = &self.lists.lists;
        let ctx = map.read_ctx().derive_add_ctx(self.by());
        let op = map.update(self.name.clone(), ctx, |_, ctx| PlacedOp::place(key, &ctx));
        self.lists.lists.apply(op);
    }

    /// Drops the list into `slot` of the order with it taken out.
    pub fn move_to_slot(&mut self, slot: usize) {
        let order = without(self.lists.names(), &self.name);
        if let Some(to) = at_slot(&order, slot) {
            self.move_to(to);
        }
    }

    pub fn can_undo(&self) -> bool {
        self.lists.can_undo(&self.name)
    }

    pub fn undo(&mut self) {
        let Some(inverse) = self.lists.history.get_mut(&self.name).and_then(Vec::pop) else {
            return;
        };
        self.apply_raw(inverse);
    }

    fn list(&self) -> List {
        self.lists.list(&self.name)
    }

    fn by(&self) -> DeviceId {
        self.session.device
    }

    fn apply(&mut self, group: EditGroup) {
        if let Some(inverse) = group.inverse(self.session, self.lists, &self.name) {
            self.lists
                .history
                .entry(self.name.clone())
                .or_default()
                .push(inverse);
        }
        self.apply_raw(group);
    }

    fn apply_raw(&mut self, group: EditGroup) {
        for edit in group.0 {
            self.edit(edit);
        }
    }

    fn edit(&mut self, edit: ItemEdit) {
        let list = self.list();
        let map = &self.lists.lists;
        let ctx = map.read_ctx().derive_add_ctx(self.by());
        let op = match edit {
            ItemEdit::AddItem { name, tags, key } => {
                let fresh = !list.contains(&name);
                self.update_item(name, ctx, |item, ctx| PlacedOp {
                    position: fresh.then(|| put(key, &ctx)),
                    inner: Some(item.inner.add_all(tags, ctx)),
                })
            }
            ItemEdit::MoveItem { name, key } => {
                if !list.contains(&name) {
                    return;
                }
                self.update_item(name, ctx, |_, ctx| PlacedOp::place(key, &ctx))
            }
            ItemEdit::RemoveItem { name } => {
                let rm_ctx = list.inner.get(&name).derive_rm_ctx();
                map.update(self.name.clone(), ctx, |entry, _| {
                    PlacedOp::inner(entry.inner.rm(name, rm_ctx))
                })
            }
            ItemEdit::AddTag { name, tag } => self.update_item(name, ctx, |item, ctx| {
                PlacedOp::inner(item.inner.add(tag, ctx))
            }),
            ItemEdit::RemoveTag { name, tag } => {
                let Some(item) = list.item(&name) else {
                    return;
                };
                let rm_ctx = item.inner.contains(&tag).derive_rm_ctx();
                // Through `update`, not `rm`, so the item itself stays present.
                self.update_item(name, ctx, |item, _| {
                    PlacedOp::inner(item.inner.rm(tag, rm_ctx))
                })
            }
        };
        self.lists.lists.apply(op);
    }

    fn update_item(
        &self,
        name: TrimStr,
        ctx: AddCtx<DeviceId>,
        f: impl FnOnce(&Item, AddCtx<DeviceId>) -> PlacedOp<<Tags as CmRDT>::Op>,
    ) -> ListsOp {
        self.lists
            .lists
            .update(self.name.clone(), ctx, |entry, ctx| {
                PlacedOp::inner(entry.inner.update(name, ctx, f))
            })
    }
}

/// Who is editing: runtime context for operations on the data, never stored with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    pub device: DeviceId,
}

impl Session {
    pub const fn new(device: DeviceId) -> Self {
        Self { device }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: Session = Session::new(DeviceId::new(1));

    fn list(raw: &str) -> TrimStr {
        TrimStr::new(raw).unwrap()
    }

    fn name(raw: &str) -> TrimStr {
        TrimStr::new(raw).unwrap()
    }

    fn tag(raw: &str) -> TrimStr {
        TrimStr::new(raw).unwrap()
    }

    fn create(session: &mut Lists, name: &str) {
        session.create(&ALICE, list(name), Position::End);
    }

    fn add(session: &mut Lists, in_list: &str, item: &str) {
        session
            .list_mut(&ALICE, list(in_list))
            .add_item(name(item), Position::End);
    }

    fn three_lists() -> Lists {
        let mut session = Lists::default();
        for entry in ["a", "b", "c"] {
            create(&mut session, entry);
        }
        session
    }

    fn home_with_items() -> Lists {
        let mut session = Lists::default();
        for item in ["milch", "brot", "kaffee"] {
            add(&mut session, "home", item);
        }
        session
    }

    fn item_order(session: &Lists) -> Vec<TrimStr> {
        session.list(&list("home")).item_names()
    }

    #[test]
    fn an_item_moves_before_another() {
        let mut session = home_with_items();

        session
            .list_mut(&ALICE, list("home"))
            .move_item(name("kaffee"), Position::Before(name("brot")));
        assert_eq!(
            item_order(&session),
            [name("milch"), name("kaffee"), name("brot")]
        );

        session
            .list_mut(&ALICE, list("home"))
            .move_item(name("kaffee"), Position::Before(name("milch")));
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
            session
                .list_mut(&ALICE, list("home"))
                .move_item_to_slot(name("kaffee"), slot);
            assert_eq!(
                item_order(&session),
                expected.map(name).to_vec(),
                "dropping kaffee into slot {slot}"
            );
        }
    }

    #[test]
    fn undo_of_a_tick_off_restores_the_items_place() {
        let mut session = home_with_items();

        session
            .list_mut(&ALICE, list("home"))
            .remove_item(name("brot"));
        session.list_mut(&ALICE, list("home")).undo();

        assert_eq!(
            item_order(&session),
            [name("milch"), name("brot"), name("kaffee")]
        );
    }

    #[test]
    fn re_adding_an_item_does_not_move_it() {
        let mut session = home_with_items();

        add(&mut session, "home", "milch");

        assert_eq!(
            item_order(&session),
            [name("milch"), name("brot"), name("kaffee")]
        );
    }

    #[test]
    fn removing_the_last_tag_keeps_the_item() {
        let mut session = Lists::default();
        add(&mut session, "home", "milch");
        session
            .list_mut(&ALICE, list("home"))
            .add_tag(name("milch"), tag("2x"));
        session
            .list_mut(&ALICE, list("home"))
            .remove_tag(name("milch"), tag("2x"));

        assert_eq!(
            session.list(&list("home")).items(),
            vec![(name("milch"), BTreeSet::new())]
        );
    }

    #[test]
    fn a_list_moves_before_another() {
        let mut session = three_lists();

        for (to, expected) in [
            (Position::Before(list("a")), ["c", "a", "b"]),
            (Position::Before(list("b")), ["a", "c", "b"]),
            (Position::End, ["a", "b", "c"]),
        ] {
            session.list_mut(&ALICE, list("c")).move_to(to.clone());
            assert_eq!(
                session.names(),
                expected.map(list).to_vec(),
                "after moving c to {to:?}"
            );
        }
    }

    #[test]
    fn restoring_a_deleted_list_puts_it_back_in_place() {
        let mut session = three_lists();

        session.remove(&ALICE, &list("b"));
        assert!(session.can_restore());
        session.restore_deleted(&ALICE);

        assert_eq!(session.names(), vec![list("a"), list("b"), list("c")]);
    }

    #[test]
    fn undo_restores_a_removed_list_with_its_items_and_tags() {
        let mut session = Lists::default();
        add(&mut session, "home", "milch");
        session
            .list_mut(&ALICE, list("home"))
            .add_tag(name("milch"), tag("2x"));
        session.remove(&ALICE, &list("home"));

        session.restore_deleted(&ALICE);

        assert_eq!(session.names(), vec![list("home")]);
        assert_eq!(
            session.list(&list("home")).tags(&name("milch")),
            BTreeSet::from([tag("2x")])
        );
    }

    // An edit that changed nothing costs no undo press; the next one reaches past it.
    #[test]
    fn re_adding_an_item_records_no_undo_step() {
        let mut session = Lists::default();
        add(&mut session, "home", "milch");
        add(&mut session, "home", "milch");

        session.list_mut(&ALICE, list("home")).undo();

        assert!(!session.list(&list("home")).contains(&name("milch")));
        assert!(!session.list_mut(&ALICE, list("home")).can_undo());
    }
}
