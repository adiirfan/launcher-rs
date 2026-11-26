//! The Albums screen's list, as the mockup orders it: Camera, then
//! Favourites and Videos when they have anything, then every other folder
//! by name.

use std::collections::{HashMap, HashSet};

use crate::arrange::Filter;
use crate::media::{Kind, MediaItem};

#[derive(Clone, Debug, PartialEq)]
pub struct Album {
    /// What opening it shows: `Bucket(folder id)`, `Favourites` or `Videos`.
    pub filter: Filter,
    pub name: String,
    pub count: u32,
    /// The newest item's id, by when it was taken.
    pub cover: i64,
}

/// Count and newest item of one album as it's built up.
struct Tally {
    count: u32,
    cover: i64,
    newest: i64,
}

impl Tally {
    fn add(&mut self, it: &MediaItem) {
        self.count += 1;
        if self.count == 1 || it.when_taken() > self.newest {
            self.newest = it.when_taken();
            self.cover = it.id;
        }
    }
}

fn empty() -> Tally {
    Tally { count: 0, cover: 0, newest: 0 }
}

pub fn albums(items: &[MediaItem], hidden: &HashSet<i64>) -> Vec<Album> {
    let (mut fav, mut vid) = (empty(), empty());
    let mut folders: HashMap<i64, (Tally, &str)> = HashMap::new();
    for it in items.iter().filter(|it| !hidden.contains(&it.bucket_id)) {
        if it.favourite {
            fav.add(it);
        }
        if it.kind == Kind::Video {
            vid.add(it);
        }
        folders.entry(it.bucket_id).or_insert_with(|| (empty(), it.folder.as_str())).0.add(it);
    }
    let mut real: Vec<Album> = folders
        .into_iter()
        .map(|(id, (t, folder))| Album {
            filter: Filter::Bucket(id),
            name: if folder.is_empty() { "Phone storage".into() } else { folder.into() },
            count: t.count,
            cover: t.cover,
        })
        .collect();
    real.sort_by_cached_key(|a| (a.name != "Camera", a.name.to_lowercase()));
    let at = usize::from(real.first().is_some_and(|a| a.name == "Camera"));
    let virtual_albums = [(fav, Filter::Favourites, "Favourites"), (vid, Filter::Videos, "Videos")]
        .into_iter()
        .filter(|(t, ..)| t.count > 0)
        .map(|(t, filter, name)| Album { filter, name: name.into(), count: t.count, cover: t.cover });
    real.splice(at..at, virtual_albums);
    real
}

/// The folders something can be moved into: every real folder (hidden
/// ones too), Camera first, then by name.
pub fn folders(items: &[MediaItem]) -> Vec<Album> {
    albums(items, &HashSet::new()).into_iter().filter(|a| matches!(a.filter, Filter::Bucket(_))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, bucket: i64, folder: &str, taken: i64) -> MediaItem {
        MediaItem {
            id,
            kind: Kind::Image,
            w: 100,
            h: 100,
            orientation: 0,
            duration_ms: 0,
            size: 1000,
            taken_ms: taken,
            modified_ms: taken,
            added_ms: taken,
            favourite: false,
            bucket_id: bucket,
            name: format!("{id}.jpg"),
            folder: folder.into(),
            mime: "image/jpeg".into(),
            expires_ms: 0,
        }
    }

    fn names(a: &[Album]) -> Vec<&str> {
        a.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn camera_first_then_names() {
        let items = [item(1, 10, "screenshots", 1), item(2, 20, "Download", 2), item(3, 30, "Camera", 3), item(4, 40, "WhatsApp Images", 4)];
        assert_eq!(names(&albums(&items, &HashSet::new())), ["Camera", "Download", "screenshots", "WhatsApp Images"]);
    }

    #[test]
    fn counts_and_newest_cover() {
        let mut items = vec![item(1, 10, "Camera", 100), item(2, 10, "Camera", 300), item(3, 10, "Camera", 200), item(4, 20, "Download", 50)];
        items[1].kind = Kind::Video;
        items[3].favourite = true;
        // No taken date: when it arrived counts instead.
        items[2].taken_ms = 0;
        items[2].added_ms = 900;
        let a = albums(&items, &HashSet::new());
        // As the mockup: Camera, then Favourites and Videos, then the rest.
        assert_eq!(names(&a), ["Camera", "Favourites", "Videos", "Download"]);
        assert_eq!((a[0].filter.clone(), a[0].count, a[0].cover), (Filter::Bucket(10), 3, 3));
        assert_eq!((a[1].filter.clone(), a[1].count, a[1].cover), (Filter::Favourites, 1, 4));
        assert_eq!((a[2].filter.clone(), a[2].count, a[2].cover), (Filter::Videos, 1, 2));
        assert_eq!((a[3].count, a[3].cover), (1, 4));
    }

    #[test]
    fn folders_leave_out_favourites_and_videos() {
        let mut items = vec![item(1, 10, "Camera", 1), item(2, 20, "Download", 2), item(3, 30, "Secret", 3)];
        items[0].favourite = true;
        items[1].kind = Kind::Video;
        let f = folders(&items);
        // Only real folders a file can go into, Camera first; hidden ones too.
        assert_eq!(names(&f), ["Camera", "Download", "Secret"]);
        assert_eq!(f[2].filter, Filter::Bucket(30));
    }

    #[test]
    fn hidden_left_out() {
        let mut items = vec![item(1, 10, "Camera", 1), item(2, 20, "Secret", 2), item(3, 20, "Secret", 3)];
        items[1].favourite = true;
        items[2].kind = Kind::Video;
        let hidden = HashSet::from([20]);
        let a = albums(&items, &hidden);
        // Hidden folders don't feed Favourites or Videos either.
        assert_eq!(names(&a), ["Camera"]);
        assert!(albums(&[], &hidden).is_empty());
    }
}
