use std::collections::BTreeSet;

use llovlist_core::{DeviceId, Lists, Position, Session, TrimStr};

const ALICE: Session = Session::new(DeviceId::new(1));
const BOB: Session = Session::new(DeviceId::new(2));

fn list(raw: &str) -> TrimStr {
    TrimStr::new(raw).unwrap()
}

fn name(raw: &str) -> TrimStr {
    TrimStr::new(raw).unwrap()
}

fn tag(raw: &str) -> TrimStr {
    TrimStr::new(raw).unwrap()
}

fn tags(raw: &[&str]) -> BTreeSet<TrimStr> {
    raw.iter().map(|raw| tag(raw)).collect()
}

fn home() -> TrimStr {
    list("home")
}

fn create(lists: &mut Lists, session: &Session, name: &str) {
    lists.create(session, list(name), Position::End);
}

fn add(lists: &mut Lists, session: &Session, in_list: &TrimStr, item: &str) {
    lists
        .list_mut(session, in_list.clone())
        .add_item(name(item), Position::End);
}

fn add_tag(lists: &mut Lists, session: &Session, item: &str, value: &str) {
    lists
        .list_mut(session, home())
        .add_tag(name(item), tag(value));
}

fn remove_item(lists: &mut Lists, session: &Session, item: &str) {
    lists.list_mut(session, home()).remove_item(name(item));
}

#[test]
fn an_empty_list_survives_a_merge() {
    let mut alice = Lists::default();
    create(&mut alice, &ALICE, "home");

    let mut bob = Lists::default();
    bob.merge(alice);

    assert_eq!(bob.names(), vec![home()]);
    assert_eq!(bob.list(&home()).items(), vec![]);
}

#[test]
fn a_tagless_item_survives_a_merge() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");

    let mut bob = Lists::default();
    bob.merge(alice);

    assert_eq!(bob.list(&home()).items(), vec![(name("Milch"), tags(&[]))]);
}

#[test]
fn item_order_survives_a_merge() {
    let mut alice = Lists::default();
    for item in ["milch", "brot", "kaffee"] {
        add(&mut alice, &ALICE, &home(), item);
    }

    let mut bob = Lists::default();
    bob.merge(alice.clone());

    assert_eq!(
        bob.list(&home()).item_names(),
        vec![name("milch"), name("brot"), name("kaffee")]
    );
    assert_eq!(
        bob.list(&home()).item_names(),
        alice.list(&home()).item_names()
    );
}

// Both positions survive, and taking the lowest makes every replica agree even though one
// device's intent is lost.
#[test]
fn concurrent_item_moves_agree_on_a_single_order() {
    let mut alice = Lists::default();
    for item in ["a", "b", "c"] {
        add(&mut alice, &ALICE, &home(), item);
    }
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice
        .list_mut(&ALICE, home())
        .move_item(name("c"), Position::Before(name("a")));
    bob.list_mut(&BOB, home())
        .move_item(name("c"), Position::Before(name("b")));

    let mut merged_one = alice.clone();
    merged_one.merge(bob.clone());
    let mut merged_two = bob;
    merged_two.merge(alice);

    assert_eq!(
        merged_one.list(&home()).item_names(),
        merged_two.list(&home()).item_names()
    );
}

#[test]
fn a_concurrent_add_beats_a_tick_off() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    remove_item(&mut alice, &ALICE, "Milch");
    add(&mut bob, &BOB, &home(), "Milch");
    alice.merge(bob);

    assert!(alice.list(&home()).contains(&name("Milch")));
}

// A deleted list comes back if another device touched it concurrently, and like a
// ticked-off item it keeps only what that device contributed.
#[test]
fn deleting_a_list_loses_to_a_concurrent_item_add() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.remove(&ALICE, &home());
    add(&mut bob, &BOB, &home(), "Brot");
    alice.merge(bob);

    assert_eq!(alice.names(), vec![home()]);
    assert_eq!(alice.list(&home()).items(), vec![(name("Brot"), tags(&[]))]);
}

#[test]
fn deleting_a_list_removes_it_everywhere_once_seen() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice.remove(&ALICE, &home());
    bob.merge(alice);

    assert_eq!(bob.names(), vec![]);
}

#[test]
fn a_tick_off_wipes_the_tags_it_had_seen_but_not_concurrent_ones() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");
    add_tag(&mut alice, &ALICE, "Milch", "2x");
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    remove_item(&mut alice, &ALICE, "Milch");
    add_tag(&mut bob, &BOB, "Milch", "3x");
    alice.merge(bob);

    assert_eq!(
        alice.list(&home()).items(),
        vec![(name("Milch"), tags(&["3x"]))]
    );
}

#[test]
fn a_tag_removed_while_the_other_device_adds_it_survives() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");
    add_tag(&mut alice, &ALICE, "Milch", "2x");
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice
        .list_mut(&ALICE, home())
        .remove_tag(name("Milch"), tag("2x"));
    add_tag(&mut bob, &BOB, "Milch", "2x");
    alice.merge(bob);

    assert_eq!(alice.list(&home()).tags(&name("Milch")), tags(&["2x"]));
}

#[test]
fn concurrent_tag_edits_keep_both_values() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    add_tag(&mut alice, &ALICE, "Milch", "2x");
    add_tag(&mut bob, &BOB, "Milch", "3x");
    alice.merge(bob);

    assert_eq!(
        alice.list(&home()).tags(&name("Milch")),
        tags(&["2x", "3x"])
    );
}

#[test]
fn tagging_an_item_nobody_added_creates_it_and_its_list() {
    let mut alice = Lists::default();

    add_tag(&mut alice, &ALICE, "Milch", "2x");

    assert_eq!(alice.names(), vec![home()]);
    assert_eq!(
        alice.list(&home()).items(),
        vec![(name("Milch"), tags(&["2x"]))]
    );
}

#[test]
fn re_adding_a_ticked_off_item_does_not_bring_its_tags_back() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");
    add_tag(&mut alice, &ALICE, "Milch", "2x");

    remove_item(&mut alice, &ALICE, "Milch");
    add(&mut alice, &ALICE, &home(), "Milch");

    assert_eq!(
        alice.list(&home()).items(),
        vec![(name("Milch"), tags(&[]))]
    );
}

#[test]
fn a_stale_replica_does_not_resurrect_a_removed_item() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");
    let stale = alice.clone();

    remove_item(&mut alice, &ALICE, "Milch");
    alice.merge(stale);

    assert!(!alice.list(&home()).contains(&name("Milch")));
}

#[test]
fn concurrent_moves_of_one_list_agree_on_a_single_order() {
    let mut alice = Lists::default();
    for name in ["a", "b", "c"] {
        create(&mut alice, &ALICE, name);
    }
    let mut bob = Lists::default();
    bob.merge(alice.clone());

    alice
        .list_mut(&ALICE, list("c"))
        .move_to(Position::Before(list("a")));
    bob.list_mut(&BOB, list("c"))
        .move_to(Position::Before(list("b")));

    let mut merged_one = alice.clone();
    merged_one.merge(bob.clone());
    let mut merged_two = bob;
    merged_two.merge(alice);

    assert_eq!(merged_one.names(), merged_two.names());
}

#[test]
fn a_list_created_without_a_position_sorts_last() {
    let mut alice = Lists::default();
    create(&mut alice, &ALICE, "a");
    // Tagging an unknown item creates its list, and that path mints no key.
    alice
        .list_mut(&ALICE, list("ghost"))
        .add_tag(name("Milch"), tag("2x"));

    assert_eq!(alice.names(), vec![list("a"), list("ghost")]);
}

// Asserts the loss a duplicated DeviceId causes; not a behaviour to repair.
#[test]
fn sharing_a_device_id_between_replicas_loses_edits() {
    let mut alice = Lists::default();
    add(&mut alice, &ALICE, &home(), "Milch");
    let mut clone_of_alice = Lists::default();
    add(&mut clone_of_alice, &ALICE, &home(), "Brot");

    alice.merge(clone_of_alice);

    assert_eq!(alice.list(&home()).items(), vec![]);
}
