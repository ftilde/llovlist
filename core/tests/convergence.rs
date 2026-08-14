use std::collections::BTreeSet;

use kauf_core::{DeviceId, Edit, ItemName, ListName, Lists, Tag};
use proptest::prelude::*;

const REPLICAS: usize = 3;

// Small pools of names and tags, so random scripts actually collide.
fn any_edit() -> impl Strategy<Value = Edit> {
    let list = (0u8..2).prop_map(|i| ListName::new(&format!("list{i}")).unwrap());
    let name = (0u8..3).prop_map(|i| ItemName::new(&format!("item{i}")).unwrap());
    let tag = (0u8..3).prop_map(|i| Tag::new(&format!("tag{i}")).unwrap());
    prop_oneof![
        list.clone().prop_map(|list| Edit::CreateList { list }),
        list.clone().prop_map(|list| Edit::RemoveList { list }),
        (list.clone(), name.clone()).prop_map(|(list, name)| Edit::add_item(list, name)),
        (list.clone(), name.clone()).prop_map(|(list, name)| Edit::RemoveItem { list, name }),
        (list.clone(), name.clone(), tag.clone()).prop_map(|(list, name, tag)| Edit::AddTag {
            list,
            name,
            tag
        }),
        (list, name, tag).prop_map(|(list, name, tag)| Edit::RemoveTag { list, name, tag }),
    ]
}

type View = Vec<(ListName, Vec<(ItemName, BTreeSet<Tag>)>)>;

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
    fn replicas_converge(script in prop::collection::vec((0..REPLICAS, any_edit()), 1..40)) {
        let mut replicas = vec![Lists::default(); REPLICAS];
        for (replica, edit) in script {
            replicas[replica].edit(DeviceId::new(replica as u64), edit);
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
