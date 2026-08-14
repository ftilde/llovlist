use std::collections::BTreeSet;

use kauf_core::{DeviceId, Edit, ItemName, ShoppingList, Tag};

const ALICE: DeviceId = DeviceId::new(1);
const BOB: DeviceId = DeviceId::new(2);

fn name(raw: &str) -> ItemName {
    ItemName::new(raw).unwrap()
}

fn tag(raw: &str) -> Tag {
    Tag::new(raw).unwrap()
}

fn tags(raw: &[&str]) -> BTreeSet<Tag> {
    raw.iter().map(|raw| tag(raw)).collect()
}

#[test]
fn a_tagless_item_survives_a_merge() {
    let mut alice = ShoppingList::default();
    alice.edit(ALICE, Edit::add_item(name("Milch")));

    let mut bob = ShoppingList::default();
    bob.merge(alice);

    assert_eq!(bob.items(), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn the_same_item_added_on_both_devices_becomes_one_item() {
    let mut alice = ShoppingList::default();
    alice.edit(ALICE, Edit::add_item(name("Milch")));
    let mut bob = ShoppingList::default();
    bob.edit(BOB, Edit::add_item(name("Milch")));

    alice.merge(bob);

    assert_eq!(alice.items(), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn a_concurrent_add_beats_a_tick_off() {
    let mut alice = ShoppingList::default();
    alice.edit(ALICE, Edit::add_item(name("Milch")));
    let mut bob = ShoppingList::default();
    bob.merge(alice.clone());

    alice.edit(
        ALICE,
        Edit::RemoveItem {
            name: name("Milch"),
        },
    );
    bob.edit(BOB, Edit::add_item(name("Milch")));
    alice.merge(bob);

    assert!(alice.contains(&name("Milch")));
}

#[test]
fn a_tick_off_wipes_the_tags_it_had_seen_but_not_concurrent_ones() {
    let mut alice = ShoppingList::default();
    alice.edit(ALICE, Edit::add_item(name("Milch")));
    alice.edit(
        ALICE,
        Edit::AddTag {
            name: name("Milch"),
            tag: tag("2x"),
        },
    );
    let mut bob = ShoppingList::default();
    bob.merge(alice.clone());

    alice.edit(
        ALICE,
        Edit::RemoveItem {
            name: name("Milch"),
        },
    );
    bob.edit(
        BOB,
        Edit::AddTag {
            name: name("Milch"),
            tag: tag("3x"),
        },
    );
    alice.merge(bob);

    assert_eq!(alice.items(), vec![(name("Milch"), tags(&["3x"]))]);
}

#[test]
fn a_tag_removed_while_the_other_device_adds_it_survives() {
    let mut alice = ShoppingList::default();
    alice.edit(ALICE, Edit::add_item(name("Milch")));
    alice.edit(
        ALICE,
        Edit::AddTag {
            name: name("Milch"),
            tag: tag("2x"),
        },
    );
    let mut bob = ShoppingList::default();
    bob.merge(alice.clone());

    alice.edit(
        ALICE,
        Edit::RemoveTag {
            name: name("Milch"),
            tag: tag("2x"),
        },
    );
    bob.edit(
        BOB,
        Edit::AddTag {
            name: name("Milch"),
            tag: tag("2x"),
        },
    );
    alice.merge(bob);

    assert_eq!(alice.tags(&name("Milch")), tags(&["2x"]));
}

#[test]
fn concurrent_tag_edits_keep_both_values() {
    let mut alice = ShoppingList::default();
    alice.edit(ALICE, Edit::add_item(name("Milch")));
    let mut bob = ShoppingList::default();
    bob.merge(alice.clone());

    alice.edit(
        ALICE,
        Edit::AddTag {
            name: name("Milch"),
            tag: tag("2x"),
        },
    );
    bob.edit(
        BOB,
        Edit::AddTag {
            name: name("Milch"),
            tag: tag("3x"),
        },
    );
    alice.merge(bob);

    assert_eq!(alice.tags(&name("Milch")), tags(&["2x", "3x"]));
}

#[test]
fn tagging_an_item_nobody_added_creates_it() {
    let mut alice = ShoppingList::default();

    alice.edit(
        ALICE,
        Edit::AddTag {
            name: name("Milch"),
            tag: tag("2x"),
        },
    );

    assert_eq!(alice.items(), vec![(name("Milch"), tags(&["2x"]))]);
}

#[test]
fn re_adding_a_ticked_off_item_does_not_bring_its_tags_back() {
    let mut alice = ShoppingList::default();
    alice.edit(ALICE, Edit::add_item(name("Milch")));
    alice.edit(
        ALICE,
        Edit::AddTag {
            name: name("Milch"),
            tag: tag("2x"),
        },
    );

    alice.edit(
        ALICE,
        Edit::RemoveItem {
            name: name("Milch"),
        },
    );
    alice.edit(ALICE, Edit::add_item(name("Milch")));

    assert_eq!(alice.items(), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn a_stale_replica_does_not_resurrect_a_removed_item() {
    let mut alice = ShoppingList::default();
    alice.edit(ALICE, Edit::add_item(name("Milch")));
    let stale = alice.clone();

    alice.edit(
        ALICE,
        Edit::RemoveItem {
            name: name("Milch"),
        },
    );
    alice.merge(stale);

    assert!(!alice.contains(&name("Milch")));
}

// Asserts the loss a duplicated DeviceId causes; not a behaviour to repair.
#[test]
fn sharing_a_device_id_between_replicas_loses_edits() {
    let mut alice = ShoppingList::default();
    alice.edit(ALICE, Edit::add_item(name("Milch")));
    let mut clone_of_alice = ShoppingList::default();
    clone_of_alice.edit(ALICE, Edit::add_item(name("Brot")));

    alice.merge(clone_of_alice);

    assert_eq!(alice.items(), vec![]);
}
