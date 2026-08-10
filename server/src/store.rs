use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use kauf_core::ShoppingList;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct ListId(String);

impl ListId {
    // Doubles as a file name, so the tight character set is what keeps an id from
    // escaping the state directory.
    pub fn new(raw: &str) -> Result<Self, InvalidListId> {
        let allowed = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_';
        if (1..=64).contains(&raw.len()) && raw.bytes().all(allowed) {
            Ok(Self(raw.to_owned()))
        } else {
            Err(InvalidListId)
        }
    }
}

impl TryFrom<String> for ListId {
    type Error = InvalidListId;

    fn try_from(raw: String) -> Result<Self, InvalidListId> {
        Self::new(&raw)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidListId;

impl fmt::Display for InvalidListId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("list names may only contain letters, digits, '-' and '_'")
    }
}

impl std::error::Error for InvalidListId {}

pub struct Store {
    dir: PathBuf,
    lists: Mutex<HashMap<ListId, ShoppingList>>,
}

impl Store {
    pub fn open(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self { dir, lists: Mutex::default() })
    }

    pub fn sync(&self, id: &ListId, incoming: ShoppingList) -> io::Result<ShoppingList> {
        let mut lists = self.lists.lock().expect("store lock poisoned");
        if !lists.contains_key(id) {
            lists.insert(id.clone(), self.load(id)?);
        }
        let stored = lists.get_mut(id).expect("just inserted");

        stored.merge(incoming);
        let merged = stored.clone();
        self.write(id, &merged)?;
        Ok(merged)
    }

    fn path(&self, id: &ListId) -> PathBuf {
        self.dir.join(format!("{}.json", id.0))
    }

    fn load(&self, id: &ListId) -> io::Result<ShoppingList> {
        match fs::read(self.path(id)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ShoppingList::default()),
            Err(error) => Err(error),
        }
    }

    fn write(&self, id: &ListId, list: &ShoppingList) -> io::Result<()> {
        let path = self.path(id);
        let temporary = path.with_extension("json.new");
        fs::write(&temporary, serde_json::to_vec(list).map_err(io::Error::other)?)?;
        fs::rename(temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use kauf_core::{DeviceId, Edit, ItemName};

    use super::*;

    fn list_with(device: u64, item: &str) -> ShoppingList {
        let mut list = ShoppingList::default();
        list.edit(DeviceId::new(device), Edit::add_item(ItemName::new(item).unwrap()));
        list
    }

    #[test]
    fn ids_that_could_escape_the_state_directory_are_rejected() {
        for raw in ["", "..", "../secrets", "a/b", "a.json", "hä", &"x".repeat(65)] {
            assert_eq!(ListId::new(raw), Err(InvalidListId), "accepted {raw:?}");
        }
        assert!(ListId::new("family_list-2").is_ok());
    }

    #[test]
    fn lists_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let id = ListId::new("home").unwrap();

        Store::open(dir.path().to_owned()).unwrap().sync(&id, list_with(1, "Milch")).unwrap();
        let restarted = Store::open(dir.path().to_owned()).unwrap();
        let merged = restarted.sync(&id, ShoppingList::default()).unwrap();

        assert!(merged.contains(&ItemName::new("Milch").unwrap()));
    }
}
