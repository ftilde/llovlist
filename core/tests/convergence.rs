use std::collections::BTreeSet;

use llovlist_core::{DeviceId, Edit, Lists, TrimStr};
use proptest::prelude::*;

const REPLICAS: usize = 3;

// Positions depend on the state they are minted against, so the script holds intents and
// each replica turns them into edits itself -- as the UI does.
#[derive(Debug, Clone)]
enum Action {
    CreateList(TrimStr),
    RemoveList(TrimStr),
    MoveListBefore(TrimStr, usize),
    AddItem(TrimStr, TrimStr),
    RemoveItem(TrimStr, TrimStr),
    MoveItemBefore(TrimStr, TrimStr, usize),
    AddTag(TrimStr, TrimStr, TrimStr),
    RemoveTag(TrimStr, TrimStr, TrimStr),
}

fn any_action() -> impl Strategy<Value = Action> {
    let list = (0u8..3).prop_map(|i| TrimStr::new(&format!("list{i}")).unwrap());
    let name = (0u8..3).prop_map(|i| TrimStr::new(&format!("item{i}")).unwrap());
    let tag = (0u8..3).prop_map(|i| TrimStr::new(&format!("tag{i}")).unwrap());
    prop_oneof![
        list.clone().prop_map(Action::CreateList),
        list.clone().prop_map(Action::RemoveList),
        (list.clone(), 0usize..4).prop_map(|(l, at)| Action::MoveListBefore(l, at)),
        (list.clone(), name.clone()).prop_map(|(l, n)| Action::AddItem(l, n)),
        (list.clone(), name.clone()).prop_map(|(l, n)| Action::RemoveItem(l, n)),
        (list.clone(), name.clone(), 0usize..4)
            .prop_map(|(l, n, at)| Action::MoveItemBefore(l, n, at)),
        (list.clone(), name.clone(), tag.clone()).prop_map(|(l, n, t)| Action::AddTag(l, n, t)),
        (list, name, tag).prop_map(|(l, n, t)| Action::RemoveTag(l, n, t)),
    ]
}

fn to_edit(lists: &Lists, by: DeviceId, action: Action) -> Edit {
    match action {
        Action::CreateList(list) => Edit::CreateList {
            key: lists.key_at_end(by),
            list,
        },
        Action::RemoveList(list) => Edit::RemoveList { list },
        Action::MoveListBefore(list, at) => {
            let names = lists.names();
            let before = names.get(at);
            let after = at.checked_sub(1).and_then(|index| names.get(index));
            Edit::MoveList {
                key: lists.key_between(by, after, before),
                list,
            }
        }
        Action::AddItem(list, name) => {
            let key = lists.item_key_at_end(by, &list);
            Edit::add_item(list, name, key)
        }
        Action::RemoveItem(list, name) => Edit::RemoveItem { list, name },
        Action::MoveItemBefore(list, name, at) => {
            let items: Vec<TrimStr> = lists
                .items(&list)
                .into_iter()
                .map(|(name, _)| name)
                .collect();
            let before = items.get(at);
            let after = at.checked_sub(1).and_then(|index| items.get(index));
            Edit::MoveItem {
                key: lists.item_key_between(by, &list, after, before),
                list,
                name,
            }
        }
        Action::AddTag(list, name, tag) => Edit::AddTag { list, name, tag },
        Action::RemoveTag(list, name, tag) => Edit::RemoveTag { list, name, tag },
    }
}

type View = Vec<(TrimStr, Vec<(TrimStr, BTreeSet<TrimStr>)>)>;

// names() and items() are both ordered, so comparing views compares the order every
// replica shows, not just the contents.
fn view(lists: &Lists) -> View {
    lists
        .names()
        .into_iter()
        .map(|name| {
            let items = lists.items(&name);
            (name, items)
        })
        .collect()
}

proptest! {
    #[test]
    fn replicas_converge(script in prop::collection::vec((0..REPLICAS, any_action()), 1..40)) {
        let mut replicas = vec![Lists::default(); REPLICAS];
        for (replica, action) in script {
            let by = DeviceId::new(replica as u64);
            let edit = to_edit(&replicas[replica], by, action);
            replicas[replica].edit(by, edit);
        }

        // Two rounds: a change must also reach replicas that only observe it second-hand.
        for _ in 0..2 {
            let snapshot = replicas.clone();
            for replica in &mut replicas {
                for other in &snapshot {
                    replica.merge(other.clone());
                }
            }
        }

        for replica in &replicas[1..] {
            prop_assert_eq!(view(replica), view(&replicas[0]));
        }
    }
}
