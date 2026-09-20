use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use llovlist_core::Data;

pub struct Store {
    path: PathBuf,
    data: Mutex<Data>,
}

impl Store {
    pub fn open(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        let path = dir.join("lists.json");
        let data = match fs::read_to_string(&path) {
            Ok(json) => Data::from_json(&json).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Data::default(),
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }

    pub fn sync(&self, incoming: Data) -> io::Result<Data> {
        let mut data = self.data.lock().expect("store lock poisoned");
        data.merge(incoming);
        let merged = data.clone();
        self.write(&merged)?;
        Ok(merged)
    }

    fn write(&self, data: &Data) -> io::Result<()> {
        let temporary = self.path.with_extension("json.new");
        fs::write(
            &temporary,
            serde_json::to_vec(data).map_err(io::Error::other)?,
        )?;
        fs::rename(temporary, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use llovlist_core::{DeviceId, Position, Session, TrimStr};

    use super::*;

    #[test]
    fn lists_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut local = Data::default();
        local
            .lists
            .list_mut(
                &Session::new(DeviceId::new(1)),
                TrimStr::new("home").unwrap(),
            )
            .add_item(TrimStr::new("Milch").unwrap(), Position::End);

        Store::open(dir.path().to_owned())
            .unwrap()
            .sync(local)
            .unwrap();
        let restarted = Store::open(dir.path().to_owned()).unwrap();
        let merged = restarted.sync(Data::default()).unwrap();

        assert!(
            merged
                .lists
                .list(&TrimStr::new("home").unwrap())
                .contains(&TrimStr::new("Milch").unwrap())
        );
    }
}
