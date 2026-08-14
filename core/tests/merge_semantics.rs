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

fn create(lists: &Lists, by: DeviceId, name: &str) -> Edit {
    Edit::CreateList {
        key: lists.key_at_end(by),
        list: list(name),
    }
}

fn add(lists: &Lists, by: DeviceId, in_list: ListName, item: &str) -> Edit {
    let key = lists.item_key_at_end(by, &in_list);
    Edit::add_item(in_list, name(item), key)
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

fn item_names(lists: &Lists, list: &ListName) -> Vec<ItemName> {
    lists
        .items(list)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

#[test]
fn an_empty_list_survives_a_merge() {
    let mut alice = Lists::default();
    let edit = create(&alice, ALICE, "home");
    alice.edit(ALICE, edit);

    let mut bob = Lists::default();
    bob.merge(alice);

    assert_eq!(bob.names(), vec![home()]);
    assert_eq!(bob.items(&home()), vec![]);
}

#[test]
fn a_tagless_item_survives_a_merge() {
    let mut alice = Lists::default();
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);

    let mut bob = Lists::default();
    bob.merge(alice);

    assert_eq!(bob.items(&home()), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn lists_created_on_both_devices_both_appear() {
    let mut alice = Lists::default();
    let edit = add(&alice, ALICE, list("home"), "Milch");
    alice.edit(ALICE, edit);
    let mut bob = Lists::default();
    let edit = add(&bob, BOB, list("baumarkt"), "Schrauben");
    bob.edit(BOB, edit);

    alice.merge(bob);

    assert_eq!(alice.names().len(), 2);
    assert!(alice.has(&list("home")) && alice.has(&list("baumarkt")));
}

#[test]
fn the_same_item_added_on_both_devices_becomes_one_item() {
    let mut alice = Lists::default();
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
    let mut bob = Lists::default();
    let edit = add(&bob, BOB, home(), "Milch");
    bob.edit(BOB, edit);

    alice.merge(bob);

    assert_eq!(alice.items(&home()), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn item_order_survives_a_merge() {
    let mut alice = Lists::default();
    for item in ["milch", "brot", "kaffee"] {
        let edit = add(&alice, ALICE, home(), item);
        alice.edit(ALICE, edit);
    }

    let mut bob = Lists::default();
    bob.merge(alice.clone());

    assert_eq!(
        item_names(&bob, &home()),
        vec![name("milch"), name("brot"), name("kaffee")]
    );
    assert_eq!(item_names(&bob, &home()), item_names(&alice, &home()));
}

// Both positions survive, and taking the lowest makes every replica agree even though one
// device's intent is lost.
#[test]
fn concurrent_item_moves_agree_on_a_single_order() {
    let mut alice = Lists::default();
    for item in ["a", "b", "c"] {
        let edit = add(&alice, ALICE, home(), item);
        alice.edit(ALICE, edit);
    }
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    let to_front = alice.item_key_between(ALICE, &home(), None, Some(&name("a")));
    alice.edit(
        ALICE,
        Edit::MoveItem {
            list: home(),
            name: name("c"),
            key: to_front,
        },
    );
    let between = bob.item_key_between(BOB, &home(), Some(&name("a")), Some(&name("b")));
    bob.edit(
        BOB,
        Edit::MoveItem {
            list: home(),
            name: name("c"),
            key: between,
        },
    );

    let mut merged_one = alice.clone();
    merged_one.merge(bob.clone());
    let mut merged_two = bob;
    merged_two.merge(alice);

    assert_eq!(
        item_names(&merged_one, &home()),
        item_names(&merged_two, &home())
    );
}

#[test]
fn a_concurrent_add_beats_a_tick_off() {
    let mut alice = Lists::default();
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.edit(ALICE, remove_item(home(), "Milch"));
    let edit = add(&bob, BOB, home(), "Milch");
    bob.edit(BOB, edit);
    alice.merge(bob);

    assert!(alice.contains(&home(), &name("Milch")));
}

// A deleted list comes back if another device touched it concurrently, and like a
// ticked-off item it keeps only what that device contributed.
#[test]
fn deleting_a_list_loses_to_a_concurrent_item_add() {
    let mut alice = Lists::default();
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.edit(ALICE, Edit::RemoveList { list: home() });
    let edit = add(&bob, BOB, home(), "Brot");
    bob.edit(BOB, edit);
    alice.merge(bob);

    assert_eq!(alice.names(), vec![home()]);
    assert_eq!(alice.items(&home()), vec![(name("Brot"), tags(&[]))]);
}

#[test]
fn deleting_a_list_removes_it_everywhere_once_seen() {
    let mut alice = Lists::default();
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.edit(ALICE, Edit::RemoveList { list: home() });
    bob.merge(alice);

    assert_eq!(bob.names(), vec![]);
}

#[test]
fn a_tick_off_wipes_the_tags_it_had_seen_but_not_concurrent_ones() {
    let mut alice = Lists::default();
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
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
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
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
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
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
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
    alice.edit(ALICE, add_tag(home(), "Milch", "2x"));

    alice.edit(ALICE, remove_item(home(), "Milch"));
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);

    assert_eq!(alice.items(&home()), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn a_stale_replica_does_not_resurrect_a_removed_item() {
    let mut alice = Lists::default();
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
    let stale = alice.clone();

    alice.edit(ALICE, remove_item(home(), "Milch"));
    alice.merge(stale);

    assert!(!alice.contains(&home(), &name("Milch")));
}

#[test]
fn lists_keep_the_order_they_were_given_not_alphabetical() {
    let mut alice = Lists::default();
    for name in ["zebra", "apfel", "milch"] {
        let edit = create(&alice, ALICE, name);
        alice.edit(ALICE, edit);
    }

    assert_eq!(
        alice.names(),
        vec![list("zebra"), list("apfel"), list("milch")]
    );
}

#[test]
fn a_moved_list_keeps_its_place_after_a_merge() {
    let mut alice = Lists::default();
    for name in ["a", "b", "c"] {
        let edit = create(&alice, ALICE, name);
        alice.edit(ALICE, edit);
    }

    let key = alice.key_between(ALICE, Some(&list("a")), Some(&list("b")));
    alice.edit(
        ALICE,
        Edit::MoveList {
            list: list("c"),
            key,
        },
    );

    let mut bob = Lists::default();
    bob.merge(alice.clone());

    assert_eq!(bob.names(), vec![list("a"), list("c"), list("b")]);
    assert_eq!(bob.names(), alice.names());
}

#[test]
fn concurrent_moves_of_one_list_agree_on_a_single_order() {
    let mut alice = Lists::default();
    for name in ["a", "b", "c"] {
        let edit = create(&alice, ALICE, name);
        alice.edit(ALICE, edit);
    }
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    let to_front = alice.key_between(ALICE, None, Some(&list("a")));
    alice.edit(
        ALICE,
        Edit::MoveList {
            list: list("c"),
            key: to_front,
        },
    );
    let between = bob.key_between(BOB, Some(&list("a")), Some(&list("b")));
    bob.edit(
        BOB,
        Edit::MoveList {
            list: list("c"),
            key: between,
        },
    );

    let mut merged_one = alice.clone();
    merged_one.merge(bob.clone());
    let mut merged_two = bob;
    merged_two.merge(alice);

    assert_eq!(merged_one.names(), merged_two.names());
}

#[test]
fn a_list_created_without_a_position_sorts_last() {
    let mut alice = Lists::default();
    let first = create(&alice, ALICE, "a");
    alice.edit(ALICE, first);
    // Tagging an unknown item creates its list, and that path mints no key.
    alice.edit(ALICE, add_tag(list("ghost"), "Milch", "2x"));

    assert_eq!(alice.names(), vec![list("a"), list("ghost")]);
}

// Asserts the loss a duplicated DeviceId causes; not a behaviour to repair.
#[test]
fn sharing_a_device_id_between_replicas_loses_edits() {
    let mut alice = Lists::default();
    let edit = add(&alice, ALICE, home(), "Milch");
    alice.edit(ALICE, edit);
    let mut clone_of_alice = Lists::default();
    let edit = add(&clone_of_alice, ALICE, home(), "Brot");
    clone_of_alice.edit(ALICE, edit);

    alice.merge(clone_of_alice);

    assert_eq!(alice.items(&home()), vec![]);
}
