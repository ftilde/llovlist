use std::collections::BTreeSet;

use llovlist_core::{DeviceId, Lists, Position, Session, TrimStr};
use proptest::prelude::*;

const REPLICAS: usize = 3;

// Positions depend on the state they are minted against, so the script holds intents and
// each replica turns them into edits itself -- as the UI does.
#[derive(Debug, Clone)]
enum Action {
    CreateList(TrimStr),
    RemoveList(TrimStr),
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
        (list.clone(), name.clone()).prop_map(|(l, n)| Action::AddItem(l, n)),
        (list.clone(), name.clone()).prop_map(|(l, n)| Action::RemoveItem(l, n)),
        (list.clone(), name.clone(), 0usize..4)
            .prop_map(|(l, n, at)| Action::MoveItemBefore(l, n, at)),
        (list.clone(), name.clone(), tag.clone()).prop_map(|(l, n, t)| Action::AddTag(l, n, t)),
        (list, name, tag).prop_map(|(l, n, t)| Action::RemoveTag(l, n, t)),
    ]
}

fn before(order: &[TrimStr], at: usize) -> Position {
    order
        .get(at)
        .map_or(Position::End, |next| Position::Before(next.clone()))
}

fn perform(lists: &mut Lists, session: &Session, action: Action) {
    match action {
        Action::CreateList(list) => lists.create(session, list),
        Action::RemoveList(list) => lists.remove(session, &list),
        Action::AddItem(list, name) => {
            lists.list_mut(session, list).add_item(name, Position::End);
        }
        Action::RemoveItem(list, name) => lists.list_mut(session, list).remove_item(name),
        Action::MoveItemBefore(list, name, at) => {
            let to = before(&lists.list(&list).item_names(), at);
            lists.list_mut(session, list).move_item(name, to);
        }
        Action::AddTag(list, name, tag) => lists.list_mut(session, list).add_tag(name, tag),
        Action::RemoveTag(list, name, tag) => lists.list_mut(session, list).remove_tag(name, tag),
    }
}

type View = Vec<(TrimStr, Vec<(TrimStr, BTreeSet<TrimStr>)>)>;

// items() is ordered, so comparing views compares the item order every replica shows,
// not just the contents.
fn view(lists: &Lists) -> View {
    lists
        .names()
        .into_iter()
        .map(|name| {
            let items = lists.list(&name).items();
            (name, items)
        })
        .collect()
}

proptest! {
    #[test]
    fn replicas_converge(script in prop::collection::vec((0..REPLICAS, any_action()), 1..40)) {
        let mut replicas = vec![Lists::default(); REPLICAS];
        for (replica, action) in script {
            let session = Session::new(DeviceId::new(replica as u64));
            perform(&mut replicas[replica], &session, action);
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
