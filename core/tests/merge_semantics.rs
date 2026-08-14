use std::collections::BTreeSet;

use kauf_core::{DeviceId, Edit, ItemName, ListName, Lists, Tag};

const ALICE: DeviceId = DeviceId::new(1);
const BOB: DeviceId = DeviceId::new(2);

fn list(raw: &str) -> ListName {
    ListName::new(raw).unwrap()
}

fn name(raw: &str) -> ItemName {
    ItemName::new(raw).unwrap()
}

fn tag(raw: &str) -> Tag {
    Tag::new(raw).unwrap()
}

fn tags(raw: &[&str]) -> BTreeSet<Tag> {
    raw.iter().map(|raw| tag(raw)).collect()
}

fn home() -> ListName {
    list("home")
}

fn add_tag(list: ListName, item: &str, value: &str) -> Edit {
    Edit::AddTag {
        list,
        name: name(item),
        tag: tag(value),
    }
}

fn remove_item(list: ListName, item: &str) -> Edit {
    Edit::RemoveItem {
        list,
        name: name(item),
    }
}

#[test]
fn an_empty_list_survives_a_merge() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::CreateList { list: home() });

    let mut bob = Lists::default();
    bob.merge(alice);

    assert_eq!(bob.names(), vec![home()]);
    assert_eq!(bob.items(&home()), vec![]);
}

#[test]
fn a_tagless_item_survives_a_merge() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));

    let mut bob = Lists::default();
    bob.merge(alice);

    assert_eq!(bob.items(&home()), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn lists_created_on_both_devices_both_appear() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(list("home"), name("Milch")));
    let mut bob = Lists::default();
    bob.edit(BOB, Edit::add_item(list("baumarkt"), name("Schrauben")));

    alice.merge(bob);

    assert_eq!(alice.names(), vec![list("baumarkt"), list("home")]);
}

#[test]
fn the_same_item_added_on_both_devices_becomes_one_item() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    let mut bob = Lists::default();
    bob.edit(BOB, Edit::add_item(home(), name("Milch")));

    alice.merge(bob);

    assert_eq!(alice.items(&home()), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn a_concurrent_add_beats_a_tick_off() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.edit(ALICE, remove_item(home(), "Milch"));
    bob.edit(BOB, Edit::add_item(home(), name("Milch")));
    alice.merge(bob);

    assert!(alice.contains(&home(), &name("Milch")));
}

// A deleted list comes back if another device touched it concurrently, and like a
// ticked-off item it keeps only what that device contributed.
#[test]
fn deleting_a_list_loses_to_a_concurrent_item_add() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.edit(ALICE, Edit::RemoveList { list: home() });
    bob.edit(BOB, Edit::add_item(home(), name("Brot")));
    alice.merge(bob);

    assert_eq!(alice.names(), vec![home()]);
    assert_eq!(alice.items(&home()), vec![(name("Brot"), tags(&[]))]);
}

#[test]
fn deleting_a_list_removes_it_everywhere_once_seen() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.edit(ALICE, Edit::RemoveList { list: home() });
    bob.merge(alice);

    assert_eq!(bob.names(), vec![]);
}

#[test]
fn a_tick_off_wipes_the_tags_it_had_seen_but_not_concurrent_ones() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    alice.edit(ALICE, add_tag(home(), "Milch", "2x"));
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.edit(ALICE, remove_item(home(), "Milch"));
    bob.edit(BOB, add_tag(home(), "Milch", "3x"));
    alice.merge(bob);

    assert_eq!(alice.items(&home()), vec![(name("Milch"), tags(&["3x"]))]);
}

#[test]
fn a_tag_removed_while_the_other_device_adds_it_survives() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    alice.edit(ALICE, add_tag(home(), "Milch", "2x"));
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.edit(
        ALICE,
        Edit::RemoveTag {
            list: home(),
            name: name("Milch"),
            tag: tag("2x"),
        },
    );
    bob.edit(BOB, add_tag(home(), "Milch", "2x"));
    alice.merge(bob);

    assert_eq!(alice.tags(&home(), &name("Milch")), tags(&["2x"]));
}

#[test]
fn concurrent_tag_edits_keep_both_values() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.edit(ALICE, add_tag(home(), "Milch", "2x"));
    bob.edit(BOB, add_tag(home(), "Milch", "3x"));
    alice.merge(bob);

    assert_eq!(alice.tags(&home(), &name("Milch")), tags(&["2x", "3x"]));
}

#[test]
fn tagging_an_item_nobody_added_creates_it_and_its_list() {
    let mut alice = Lists::default();

    alice.edit(ALICE, add_tag(home(), "Milch", "2x"));

    assert_eq!(alice.names(), vec![home()]);
    assert_eq!(alice.items(&home()), vec![(name("Milch"), tags(&["2x"]))]);
}

#[test]
fn re_adding_a_ticked_off_item_does_not_bring_its_tags_back() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    alice.edit(ALICE, add_tag(home(), "Milch", "2x"));

    alice.edit(ALICE, remove_item(home(), "Milch"));
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));

    assert_eq!(alice.items(&home()), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn a_stale_replica_does_not_resurrect_a_removed_item() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    let stale = alice.clone();

    alice.edit(ALICE, remove_item(home(), "Milch"));
    alice.merge(stale);

    assert!(!alice.contains(&home(), &name("Milch")));
}

// Asserts the loss a duplicated DeviceId causes; not a behaviour to repair.
#[test]
fn sharing_a_device_id_between_replicas_loses_edits() {
    let mut alice = Lists::default();
    alice.edit(ALICE, Edit::add_item(home(), name("Milch")));
    let mut clone_of_alice = Lists::default();
    clone_of_alice.edit(ALICE, Edit::add_item(home(), name("Brot")));

    alice.merge(clone_of_alice);

    assert_eq!(alice.items(&home()), vec![]);
}
