use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use llovlist_core::Lists;

pub struct Store {
    path: PathBuf,
    lists: Mutex<Lists>,
}

impl Store {
    pub fn open(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let path = dir.join("lists.json");
        let lists = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Lists::default(),
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            lists: Mutex::new(lists),
        })
    }

    pub fn sync(&self, incoming: Lists) -> io::Result<Lists> {
        let mut lists = self.lists.lock().expect("store lock poisoned");
        lists.merge(incoming);
        let merged = lists.clone();
        self.write(&merged)?;
        Ok(merged)
    }

    fn write(&self, lists: &Lists) -> io::Result<()> {
        let temporary = self.path.with_extension("json.new");
        fs::write(
            &temporary,
            serde_json::to_vec(lists).map_err(io::Error::other)?,
        )?;
        fs::rename(temporary, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use llovlist_core::{DeviceId, Edit, ItemName, ListName};

    use super::*;

    #[test]
    fn lists_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut local = Lists::default();
        let list = ListName::new("home").unwrap();
        let key = local.item_key_at_end(DeviceId::new(1), &list);
        local.edit(
            DeviceId::new(1),
            Edit::add_item(list, ItemName::new("Milch").unwrap(), key),
        );

        Store::open(dir.path().to_owned())
            .unwrap()
            .sync(local)
            .unwrap();
        let restarted = Store::open(dir.path().to_owned()).unwrap();
        let merged = restarted.sync(Lists::default()).unwrap();

        assert!(merged.contains(
            &ListName::new("home").unwrap(),
            &ItemName::new("Milch").unwrap()
        ));
    }
}
