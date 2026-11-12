use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
            Self::RemoveItem { name } if list.contains(name) => vec![list.re_add(by, name).0],
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct List {
    items: Items,
}

impl CmRDT for List {
    // `None` touches the list without changing it, which is how an empty one is created.
    type Op = Option<<Items as CmRDT>::Op>;
    type Validation = <Items as CmRDT>::Validation;

    fn validate_op(&self, op: &Self::Op) -> Result<(), Self::Validation> {
        op.as_ref().map_or(Ok(()), |op| self.items.validate_op(op))
    }

    fn apply(&mut self, op: Self::Op) {
        if let Some(op) = op {
            self.items.apply(op);
        }
    }
}

impl CvRDT for List {
    type Validation = <Items as CvRDT>::Validation;

    fn validate_merge(&self, other: &Self) -> Result<(), Self::Validation> {
        self.items.validate_merge(&other.items)
    }

    fn merge(&mut self, other: Self) {
        self.items.merge(other.items);
    }
}

impl ResetRemove<DeviceId> for List {
    fn reset_remove(&mut self, clock: &VClock<DeviceId>) {
        self.items.reset_remove(clock);
    }
}

impl List {
    pub fn item(&self, name: &TrimStr) -> Option<Item> {
        self.items.get(name).val
    }

    pub fn contains(&self, name: &TrimStr) -> bool {
        self.item(name).is_some()
    }

    pub fn tags(&self, name: &TrimStr) -> BTreeSet<TrimStr> {
        self.item(name).map(|item| item.tags()).unwrap_or_default()
    }

    pub fn items(&self) -> Vec<(TrimStr, BTreeSet<TrimStr>)> {
        let mut items: Vec<(Option<OrderKey>, TrimStr, BTreeSet<TrimStr>)> = self
            .items
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

    /// What it takes to put `name` back as it is now: tags and place.
    pub fn snapshot(&self, by: DeviceId, name: &TrimStr) -> Option<ItemSnapshot> {
        self.contains(name).then(|| self.re_add(by, name))
    }

    fn re_add(&self, by: DeviceId, name: &TrimStr) -> ItemSnapshot {
        ItemSnapshot(ItemEdit::AddItem {
            key: self.item_key(name).unwrap_or_else(|| {
                self.key_for(by, name, &Position::End)
                    .expect("End always resolves")
            }),
            name: name.clone(),
            tags: self.tags(name),
        })
    }
}

/// An item as it was, for `ListMut::restore`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSnapshot(ItemEdit);

impl ItemSnapshot {
    pub fn name(&self) -> &TrimStr {
        match &self.0 {
            ItemEdit::AddItem { name, .. } => name,
            _ => unreachable!("a snapshot is always an AddItem"),
        }
    }

    pub fn tags(&self) -> &BTreeSet<TrimStr> {
        match &self.0 {
            ItemEdit::AddItem { tags, .. } => tags,
            _ => unreachable!("a snapshot is always an AddItem"),
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

/// A name without its case and spacing, so that its variations count as one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Group(String);

impl Group {
    fn of(name: &str) -> Self {
        let words: Vec<String> = name.split_whitespace().map(str::to_lowercase).collect();
        Self(words.join(" "))
    }
}

/// How often each variation of each group was typed.
type Groups = BTreeMap<Group, BTreeMap<TrimStr, u64>>;

fn count(groups: &mut Groups, typed: &TrimStr) {
    let variations = groups.entry(Group::of(typed.as_str())).or_default();
    *variations.entry(typed.clone()).or_default() += 1;
}

fn merge_counts(into: &mut Groups, from: Groups) {
    for (group, variations) in from {
        for (variation, count) in variations {
            let held = into
                .entry(group.clone())
                .or_default()
                .entry(variation)
                .or_default();
            // Both sides counted the adds they share, so a sum would count those twice.
            *held = (*held).max(count);
        }
    }
}

/// The most typed variation of every group starting with `prefix`, most typed group first.
fn suggestions(groups: &Groups, prefix: &str) -> Vec<TrimStr> {
    let prefix = Group::of(prefix);
    let mut matching: Vec<(u64, &TrimStr)> = groups
        .iter()
        .filter(|(group, _)| group.0.starts_with(&prefix.0))
        .filter_map(|(_, variations)| {
            let (variation, _) = variations.iter().max_by_key(|(_, count)| **count)?;
            Some((variations.values().sum(), variation))
        })
        .collect();
    matching.sort_by_key(|(count, _)| Reverse(*count));
    matching
        .into_iter()
        .map(|(_, variation)| variation.clone())
        .collect()
}

/// How often each item name and tag was typed, to suggest them again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    item_names_by_list: BTreeMap<TrimStr, Groups>,
    tags_by_item: BTreeMap<Group, Groups>,
}

impl Usage {
    pub fn record_item(&mut self, list: &TrimStr, name: &TrimStr) {
        count(
            self.item_names_by_list.entry(list.clone()).or_default(),
            name,
        );
    }

    pub fn record_tag(&mut self, item: &TrimStr, tag: &TrimStr) {
        let item = Group::of(item.as_str());
        count(self.tags_by_item.entry(item).or_default(), tag);
    }

    /// Item names typed into `list` that start with `prefix`.
    pub fn suggest_items(&self, list: &TrimStr, prefix: &str) -> Vec<TrimStr> {
        self.item_names_by_list
            .get(list)
            .map(|groups| suggestions(groups, prefix))
            .unwrap_or_default()
    }

    /// Tags starting with `prefix`: those of `item`, then with a prefix those of other items.
    pub fn suggest_tags(&self, item: &TrimStr, prefix: &str) -> Vec<TrimStr> {
        let mut tags = self
            .tags_by_item
            .get(&Group::of(item.as_str()))
            .map(|groups| suggestions(groups, prefix))
            .unwrap_or_default();
        if !prefix.trim().is_empty() {
            let own: BTreeSet<Group> = tags.iter().map(|tag| Group::of(tag.as_str())).collect();
            let of_any_item = suggestions(&self.tags_of_all_items(), prefix);
            tags.extend(
                of_any_item
                    .into_iter()
                    .filter(|tag| !own.contains(&Group::of(tag.as_str()))),
            );
        }
        tags
    }

    fn tags_of_all_items(&self) -> Groups {
        let mut all = Groups::new();
        for (group, variations) in self.tags_by_item.values().flatten() {
            for (variation, count) in variations {
                let summed = all.entry(group.clone()).or_default();
                *summed.entry(variation.clone()).or_default() += count;
            }
        }
        all
    }

    pub fn merge(&mut self, other: Self) {
        for (list, groups) in other.item_names_by_list {
            merge_counts(self.item_names_by_list.entry(list).or_default(), groups);
        }
        for (item, groups) in other.tags_by_item {
            merge_counts(self.tags_by_item.entry(item).or_default(), groups);
        }
    }
}

/// The format `Data` is stored and synced in.
pub const DATA_VERSION: u32 = 1;

/// Proof that the data it came with is in the format of `DATA_VERSION`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
struct Version;

impl From<Version> for u32 {
    fn from(_: Version) -> Self {
        DATA_VERSION
    }
}

impl TryFrom<u32> for Version {
    type Error = String;

    fn try_from(version: u32) -> Result<Self, String> {
        if version == DATA_VERSION {
            Ok(Self)
        } else {
            Err(format!(
                "data is in format {version}, this build reads format {DATA_VERSION}"
            ))
        }
    }
}

/// Everything a device holds and syncs: the shopping lists, the templates to fill them
/// from, and how often what was typed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Data {
    version: Version,
    pub lists: Lists,
    pub templates: Lists,
    pub usage: Usage,
}

impl Data {
    pub fn merge(&mut self, other: Self) {
        self.lists.merge(other.lists);
        self.templates.merge(other.templates);
        self.usage.merge(other.usage);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lists {
    lists: Map<TrimStr, List, DeviceId>,
    // Undo state is per device and never leaves it.
    #[serde(skip)]
    history: HashMap<TrimStr, History>,
    #[serde(skip)]
    deleted: Vec<Removed>,
}

/// Undo steps taken and not yet taken back; a fresh edit drops the redo side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct History {
    undo: Vec<EditGroup>,
    redo: Vec<EditGroup>,
}

/// What it takes to put a removed list back: its items.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Removed {
    list: TrimStr,
    items: Vec<ItemEdit>,
}

impl Lists {
    pub fn names(&self) -> Vec<TrimStr> {
        let mut names: Vec<TrimStr> = self.lists.keys().map(|entry| entry.val.clone()).collect();
        names.sort();
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

    pub fn create(&mut self, session: &Session, name: TrimStr) {
        let ctx = self.lists.read_ctx().derive_add_ctx(session.device);
        let op = self.lists.update(name, ctx, |_, _| None);
        self.lists.apply(op);
    }

    pub fn remove(&mut self, session: &Session, name: &TrimStr) {
        // Deleting a list outlives the list, so its restore goes on its own stack --
        // and the list's own history describes a state that no longer exists.
        self.history.remove(name);
        if self.has(name) {
            self.deleted
                .push(self.removed(session.device, name.clone()));
        }
        let ctx = self.lists.get(name).derive_rm_ctx();
        let op = self.lists.rm(name.clone(), ctx);
        self.lists.apply(op);
    }

    pub fn can_undo(&self, name: &TrimStr) -> bool {
        self.history
            .get(name)
            .is_some_and(|history| !history.undo.is_empty())
    }

    pub fn can_redo(&self, name: &TrimStr) -> bool {
        self.history
            .get(name)
            .is_some_and(|history| !history.redo.is_empty())
    }

    /// The list `restore_deleted` would bring back.
    pub fn restorable(&self) -> Option<&TrimStr> {
        self.deleted.last().map(|removed| &removed.list)
    }

    /// Puts the most recently deleted list back, with its items and tags.
    pub fn restore_deleted(&mut self, session: &Session) {
        let Some(removed) = self.deleted.pop() else {
            return;
        };
        self.create(session, removed.list.clone());
        let mut list = self.list_mut(session, removed.list);
        for item in removed.items {
            list.edit(item);
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.lists.merge(other.lists);
    }

    fn removed(&self, by: DeviceId, name: TrimStr) -> Removed {
        let list = self.list(&name);
        Removed {
            items: list
                .item_names()
                .iter()
                .map(|item| list.re_add(by, item).0)
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

    /// Puts a removed item back where it was, with the tags it had.
    pub fn restore(&mut self, snapshot: ItemSnapshot) {
        self.apply(snapshot.0.into());
    }

    /// Adds every item of `template` at the end, tags included, as one undo step.
    pub fn fill_from(&mut self, template: &List) {
        let list = self.list();
        let mut last = list
            .item_names()
            .last()
            .and_then(|name| list.item_key(name));
        let edits = template
            .items()
            .into_iter()
            .map(|(name, tags)| {
                // Existing items keep their place; only their tags are topped up.
                let key = list.item_key(&name).unwrap_or_else(|| {
                    let key = OrderKey::between(self.by(), last.as_ref(), None);
                    last = Some(key.clone());
                    key
                });
                ItemEdit::AddItem { name, tags, key }
            })
            .collect();
        if let Some(group) = EditGroup::collect(edits) {
            self.apply(group);
        }
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

    pub fn can_undo(&self) -> bool {
        self.lists.can_undo(&self.name)
    }

    pub fn can_redo(&self) -> bool {
        self.lists.can_redo(&self.name)
    }

    pub fn undo(&mut self) {
        let Some(step) = self.history().undo.pop() else {
            return;
        };
        let inverse = step.inverse(self.session, self.lists, &self.name);
        self.history().redo.extend(inverse);
        self.apply_raw(step);
    }

    pub fn redo(&mut self) {
        let Some(step) = self.history().redo.pop() else {
            return;
        };
        let inverse = step.inverse(self.session, self.lists, &self.name);
        self.history().undo.extend(inverse);
        self.apply_raw(step);
    }

    fn history(&mut self) -> &mut History {
        self.lists.history.entry(self.name.clone()).or_default()
    }

    fn list(&self) -> List {
        self.lists.list(&self.name)
    }

    fn by(&self) -> DeviceId {
        self.session.device
    }

    fn apply(&mut self, group: EditGroup) {
        if let Some(inverse) = group.inverse(self.session, self.lists, &self.name) {
            let history = self.history();
            history.undo.push(inverse);
            history.redo.clear();
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
                let rm_ctx = list.items.get(&name).derive_rm_ctx();
                map.update(self.name.clone(), ctx, |entry, _| {
                    Some(entry.items.rm(name, rm_ctx))
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
                Some(entry.items.update(name, ctx, f))
            })
    }
}

/// One device's order of the lists; never synced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListOrder(Vec<TrimStr>);

impl ListOrder {
    /// The names of `lists` in this order, those it does not hold after the rest.
    pub fn arrange(&self, lists: &Lists) -> Vec<TrimStr> {
        let mut names = lists.names();
        names.sort_by_key(|name| {
            let held = self.0.iter().position(|held| held == name);
            held.unwrap_or(self.0.len())
        });
        names
    }

    /// Drops `name` into `slot` of `lists` as arranged with `name` taken out.
    pub fn move_to_slot(&mut self, lists: &Lists, name: &TrimStr, slot: usize) {
        let others = without(self.arrange(lists), name);
        if slot > others.len() {
            return;
        }
        // A deleted list keeps its place for as long as it can be restored.
        let restorable = |held: &TrimStr| lists.deleted.iter().any(|removed| &removed.list == held);
        self.0
            .retain(|held| held != name && (others.contains(held) || restorable(held)));
        for other in &others {
            if !self.0.contains(other) {
                self.0.push(other.clone());
            }
        }
        let index = others.get(slot).map_or(self.0.len(), |next| {
            let held = self.0.iter().position(|held| held == next);
            held.expect("every other name is held by now")
        });
        self.0.insert(index, name.clone());
    }

    pub fn move_to_end(&mut self, lists: &Lists, name: &TrimStr) {
        let end = without(lists.names(), name).len();
        self.move_to_slot(lists, name, end);
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

    fn add(session: &mut Lists, in_list: &str, item: &str) {
        session
            .list_mut(&ALICE, list(in_list))
            .add_item(name(item), Position::End);
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
    fn redo_puts_an_undone_tick_off_back() {
        let mut session = home_with_items();

        session
            .list_mut(&ALICE, list("home"))
            .remove_item(name("brot"));
        session.list_mut(&ALICE, list("home")).undo();
        session.list_mut(&ALICE, list("home")).redo();

        assert_eq!(item_order(&session), [name("milch"), name("kaffee")]);
        assert!(!session.can_redo(&list("home")));
        assert!(session.can_undo(&list("home")));
    }

    #[test]
    fn filling_from_a_template_appends_in_order_and_undoes_as_one_step() {
        let mut session = home_with_items();
        let mut template = Lists::default();
        for item in ["kaffee", "eier", "mehl"] {
            add(&mut template, "weekly", item);
        }
        template
            .list_mut(&ALICE, list("weekly"))
            .add_tag(name("eier"), tag("10"));

        session
            .list_mut(&ALICE, list("home"))
            .fill_from(&template.list(&list("weekly")));

        assert_eq!(
            item_order(&session),
            [
                name("milch"),
                name("brot"),
                name("kaffee"),
                name("eier"),
                name("mehl")
            ]
        );
        assert_eq!(
            session.list(&list("home")).tags(&name("eier")),
            BTreeSet::from([tag("10")])
        );

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

    fn names(raw: &[&str]) -> Vec<TrimStr> {
        raw.iter().map(|raw| list(raw)).collect()
    }

    fn lists(raw: &[&str]) -> Lists {
        let mut lists = Lists::default();
        for name in names(raw) {
            lists.create(&ALICE, name);
        }
        lists
    }

    #[test]
    fn a_list_can_be_dropped_into_any_slot_among_lists_the_order_has_not_seen() {
        let lists = lists(&["a", "b", "c"]);

        for (slot, expected) in [
            (0, ["c", "a", "b"]),
            (1, ["a", "c", "b"]),
            (2, ["a", "b", "c"]),
        ] {
            let mut order = ListOrder::default();
            order.move_to_slot(&lists, &list("c"), slot);
            assert_eq!(
                order.arrange(&lists),
                names(&expected),
                "dropping c into slot {slot}"
            );
        }
    }

    #[test]
    fn a_restored_list_gets_its_place_back() {
        let mut lists = lists(&["a", "b", "c"]);
        let mut order = ListOrder::default();
        order.move_to_slot(&lists, &list("c"), 0);

        lists.remove(&ALICE, &list("c"));
        order.move_to_slot(&lists, &list("b"), 0);
        lists.restore_deleted(&ALICE);

        assert_eq!(order.arrange(&lists), names(&["c", "b", "a"]));
    }

    #[test]
    fn a_list_gone_beyond_restoring_loses_its_place() {
        let mut order = ListOrder::default();
        order.move_to_slot(&lists(&["a", "b", "c"]), &list("c"), 0);

        order.move_to_slot(&lists(&["a", "b"]), &list("b"), 0);

        assert_eq!(
            order.arrange(&lists(&["a", "b", "c"])),
            names(&["b", "a", "c"])
        );
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

    #[test]
    fn merging_usage_again_counts_nothing_twice() {
        let mut phone = Usage::default();
        phone.record_item(&list("home"), &name("Milch"));
        phone.record_item(&list("home"), &name("Milch"));
        phone.record_tag(&name("Milch"), &tag("bio"));

        let mut server = phone.clone();
        server.merge(phone.clone());

        assert_eq!(server, phone);
    }

    #[test]
    fn a_group_ranks_by_all_its_variations_and_shows_the_most_typed_one() {
        let mut usage = Usage::default();
        for typed in ["milch", "Milch", "Milch", "Mehl", "Mehl", "Brot"] {
            usage.record_item(&list("home"), &name(typed));
        }

        assert_eq!(
            usage.suggest_items(&list("home"), "m"),
            [name("Milch"), name("Mehl")]
        );
        assert_eq!(usage.suggest_items(&list("baumarkt"), "m"), []);
    }

    #[test]
    fn an_items_own_tags_come_before_those_of_other_items() {
        let mut usage = Usage::default();
        usage.record_tag(&name("Milch"), &tag("bio"));
        for typed in ["Bio", "Bio", "bund"] {
            usage.record_tag(&name("Brot"), &tag(typed));
        }

        assert_eq!(
            usage.suggest_tags(&name("milch"), "b"),
            [tag("bio"), tag("bund")]
        );
        assert_eq!(usage.suggest_tags(&name("milch"), ""), [tag("bio")]);
    }

    #[test]
    fn data_in_another_format_or_without_a_version_is_refused() {
        let written = serde_json::to_value(Data::default()).unwrap();
        assert_eq!(written["version"], DATA_VERSION);

        let mut newer = written.clone();
        newer["version"] = (DATA_VERSION + 1).into();
        assert!(serde_json::from_value::<Data>(newer).is_err());

        let mut unversioned = written;
        unversioned.as_object_mut().unwrap().remove("version");
        assert!(serde_json::from_value::<Data>(unversioned).is_err());
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
