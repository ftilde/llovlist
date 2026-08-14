use kauf_core::{DeviceId, Edit, ItemName, ShoppingList, Tag};
use proptest::prelude::*;

const REPLICAS: usize = 3;

// Small pools of names and tags, so random scripts actually collide.
fn any_edit() -> impl Strategy<Value = Edit> {
    let name = (0u8..3).prop_map(|i| ItemName::new(&format!("item{i}")).unwrap());
    let tag = (0u8..3).prop_map(|i| Tag::new(&format!("tag{i}")).unwrap());
    prop_oneof![
        name.clone().prop_map(Edit::add_item),
        name.clone().prop_map(|name| Edit::RemoveItem { name }),
        (name.clone(), tag.clone()).prop_map(|(name, tag)| Edit::AddTag { name, tag }),
        (name, tag).prop_map(|(name, tag)| Edit::RemoveTag { name, tag }),
    ]
}

proptest! {
    #[test]
    fn replicas_converge(script in prop::collection::vec((0..REPLICAS, any_edit()), 1..40)) {
        let mut replicas = vec![ShoppingList::default(); REPLICAS];
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
            prop_assert_eq!(replica.items(), replicas[0].items());
        }
    }
}
