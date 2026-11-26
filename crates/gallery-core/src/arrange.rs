//! Sorting the library and splitting it into the grid's groups.

use std::collections::HashSet;

use crate::dates::CivilDate;
use crate::media::MediaItem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Taken,
    Changed,
    Name,
    Size,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Desc,
    Asc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Bucket(i64),
    Videos,
    Favourites,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub label: String,
    pub start: u32,
    pub len: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arranged {
    /// Indexes into the item list, in display order.
    pub order: Vec<u32>,
    pub groups: Vec<Group>,
}

fn size_band(bytes: u64) -> u8 {
    if bytes >= 20_000_000 {
        0
    } else if bytes >= 2_000_000 {
        1
    } else {
        2
    }
}

const SIZE_LABELS: [&str; 3] = ["OVER 20 MB", "2 TO 20 MB", "UNDER 2 MB"];

fn name_group(name: &str) -> String {
    match name.chars().next() {
        Some(c) if c.is_alphanumeric() => c.to_uppercase().collect(),
        _ => "#".into(),
    }
}

/// Picks the items `filter` lets through (hidden folders are left out of
/// everything but their own album), sorts them by `key` in `order` (ties:
/// newest id first, so the order never jumps about), and splits them into
/// headed groups. Dates are local: `utc_offset_s` is the phone's offset.
pub fn arrange(
    items: &[MediaItem],
    key: SortKey,
    order: Order,
    filter: &Filter,
    today: CivilDate,
    utc_offset_s: i32,
    hidden: &HashSet<i64>,
) -> Arranged {
    use crate::dates::{civil_from_days, day_label, local_day};
    use crate::media::Kind;

    let keep = |m: &MediaItem| match filter {
        Filter::Bucket(b) => m.bucket_id == *b,
        Filter::All => !hidden.contains(&m.bucket_id),
        Filter::Videos => m.kind == Kind::Video && !hidden.contains(&m.bucket_id),
        Filter::Favourites => m.favourite && !hidden.contains(&m.bucket_id),
    };
    let mut idx: Vec<u32> = (0..items.len() as u32).filter(|&i| keep(&items[i as usize])).collect();

    // Sort on a precomputed key, so names are lower-cased once each.
    let names: Vec<String> = match key {
        SortKey::Name => items.iter().map(|m| m.name.to_lowercase()).collect(),
        _ => Vec::new(),
    };
    let num = |i: u32| -> i64 {
        let m = &items[i as usize];
        match key {
            SortKey::Taken => m.when_taken(),
            SortKey::Changed => m.modified_ms,
            SortKey::Size => m.size as i64,
            SortKey::Name => 0,
        }
    };
    idx.sort_unstable_by(|&a, &b| {
        let primary = if key == SortKey::Name { names[a as usize].cmp(&names[b as usize]) } else { num(a).cmp(&num(b)) };
        let primary = if order == Order::Desc { primary.reverse() } else { primary };
        primary.then_with(|| items[b as usize].id.cmp(&items[a as usize].id))
    });

    // Group runs of the same heading.
    #[derive(PartialEq)]
    enum G {
        Day(i64),
        Text(String),
        Band(u8),
    }
    let group_of = |i: u32| -> G {
        let m = &items[i as usize];
        match key {
            SortKey::Taken => G::Day(local_day(m.when_taken(), utc_offset_s)),
            SortKey::Changed => G::Day(local_day(m.modified_ms, utc_offset_s)),
            SortKey::Name => G::Text(name_group(&m.name)),
            SortKey::Size => G::Band(size_band(m.size)),
        }
    };
    let mut groups: Vec<Group> = Vec::new();
    let mut current: Option<G> = None;
    for (pos, &i) in idx.iter().enumerate() {
        let g = group_of(i);
        if current.as_ref() == Some(&g) {
            groups.last_mut().unwrap().len += 1;
            continue;
        }
        let label = match &g {
            G::Day(day) => day_label(civil_from_days(*day), today),
            G::Text(t) => t.clone(),
            G::Band(b) => SIZE_LABELS[*b as usize].into(),
        };
        groups.push(Group { label, start: pos as u32, len: 1 });
        current = Some(g);
    }
    Arranged { order: idx, groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::days_from_civil;
    use crate::media::Kind;

    const TODAY: CivilDate = CivilDate { y: 2026, m: 11, d: 26 };

    fn ms(m: u8, d: u8, hour: i64) -> i64 {
        days_from_civil(CivilDate { y: 2026, m, d }) * 86_400_000 + hour * 3_600_000
    }

    fn item(id: i64, name: &str, taken: i64, changed: i64, size_mb: f64, kind: Kind) -> MediaItem {
        MediaItem {
            id,
            kind,
            w: 4000,
            h: 3000,
            orientation: 0,
            duration_ms: 0,
            size: (size_mb * 1_000_000.0) as u64,
            taken_ms: taken,
            modified_ms: changed,
            added_ms: taken,
            favourite: false,
            bucket_id: 1,
            name: name.into(),
            folder: "Camera".into(),
            mime: "image/jpeg".into(),
            expires_ms: 0,
        }
    }

    /// The 24 items of the approved mockup's Photos screen.
    fn mockup() -> Vec<MediaItem> {
        let raw: [(&str, u8, u8, f64, char); 24] = [
            ("IMG_20261126_093012.jpg", 26, 0, 4.1, 'p'),
            ("IMG_20261126_091544.jpg", 26, 0, 3.6, 'p'),
            ("VID_20261126_084210.mp4", 26, 0, 48.2, 'v'),
            ("Screenshot_20261126_080102.png", 26, 0, 0.9, 'p'),
            ("IMG_20261126_075530.jpg", 26, 0, 3.2, 'p'),
            ("IMG_20261125_191020.jpg", 25, 0, 4.8, 'p'),
            ("IMG_20261125_190455.jpg", 25, 0, 4.5, 'p'),
            ("IMG-20261125-WA0012.jpg", 25, 0, 0.3, 'p'),
            ("VID_20261125_172233.mp4", 25, 0, 22.6, 'v'),
            ("Screenshot_20261125_121530.png", 25, 0, 1.1, 'p'),
            ("IMG_20261123_204410.jpg", 23, 0, 5.2, 'p'),
            ("IMG_20261123_203952.jpg", 23, 26, 5.0, 'p'),
            ("reaction.gif", 23, 0, 6.8, 'g'),
            ("IMG_20261123_131202.jpg", 23, 0, 3.0, 'p'),
            ("IMG_20261123_130845.jpg", 23, 0, 2.9, 'p'),
            ("IMG_20261123_102233.jpg", 23, 0, 4.4, 'p'),
            ("VID_20261123_101540.mp4", 23, 0, 91.5, 'v'),
            ("IMG_20261121_165012.jpg", 21, 26, 4.0, 'p'),
            ("IMG_20261121_164530.jpg", 21, 0, 3.9, 'p'),
            ("IMG-20261121-WA0003.jpg", 21, 0, 0.2, 'p'),
            ("Screenshot_20261121_093011.png", 21, 0, 0.8, 'p'),
            ("IMG_20261109_180012.jpg", 9, 25, 4.6, 'p'),
            ("IMG_20261109_175833.jpg", 9, 0, 3.4, 'p'),
            ("VID_20261109_175500.mp4", 9, 0, 35.0, 'v'),
        ];
        raw.iter()
            .enumerate()
            .map(|(i, &(name, day, changed, mb, k))| {
                let kind = match k {
                    'v' => Kind::Video,
                    'g' => Kind::Gif,
                    _ => Kind::Image,
                };
                // Earlier in the list = later in the day.
                let taken = ms(11, day, 20 - i as i64 % 12);
                let changed = if changed == 0 { taken } else { ms(11, changed, 12) };
                item(100 + i as i64, name, taken, changed, mb, kind)
            })
            .collect()
    }

    fn labels(a: &Arranged) -> Vec<(String, u32)> {
        a.groups.iter().map(|g| (g.label.clone(), g.len)).collect()
    }

    fn run(items: &[MediaItem], key: SortKey, order: Order) -> Arranged {
        arrange(items, key, order, &Filter::All, TODAY, 0, &HashSet::new())
    }

    #[test]
    fn taken_groups_match_mockup() {
        let a = run(&mockup(), SortKey::Taken, Order::Desc);
        let want = [("TODAY", 5), ("YESTERDAY", 5), ("MONDAY 23 NOV", 7), ("SATURDAY 21 NOV", 4), ("MONDAY 9 NOV", 3)];
        assert_eq!(labels(&a), want.map(|(l, n)| (l.to_string(), n)).to_vec());
        assert_eq!(a.order.len(), 24);
        assert_eq!(a.order[0], 0);
        let a = run(&mockup(), SortKey::Taken, Order::Asc);
        assert_eq!(a.groups[0].label, "MONDAY 9 NOV");
        assert_eq!(a.groups[4].label, "TODAY");
    }

    #[test]
    fn changed_moves_edited_items() {
        let items = mockup();
        let a = run(&items, SortKey::Changed, Order::Desc);
        assert_eq!(a.groups[0].label, "TODAY");
        assert_eq!(a.groups[0].len, 7);
        let today: Vec<&str> = a.order[..7].iter().map(|&i| items[i as usize].name.as_str()).collect();
        assert!(today.contains(&"IMG_20261123_203952.jpg"));
        assert!(today.contains(&"IMG_20261121_165012.jpg"));
    }

    #[test]
    fn name_groups_by_letter() {
        let items = mockup();
        let a = run(&items, SortKey::Name, Order::Asc);
        let l: Vec<String> = a.groups.iter().map(|g| g.label.clone()).collect();
        assert_eq!(l, ["I", "R", "S", "V"]);
        assert_eq!(items[a.order[0] as usize].name, "IMG-20261121-WA0003.jpg");
        let a = run(&items, SortKey::Name, Order::Desc);
        let l: Vec<String> = a.groups.iter().map(|g| g.label.clone()).collect();
        assert_eq!(l, ["V", "S", "R", "I"]);
        let mut odd = items[..2].to_vec();
        odd[0].name = "_hidden.jpg".into();
        odd[1].name = "日本.jpg".into();
        let l: Vec<String> = run(&odd, SortKey::Name, Order::Asc).groups.iter().map(|g| g.label.clone()).collect();
        assert_eq!(l, ["#", "日"]);
    }

    #[test]
    fn size_bands() {
        let a = run(&mockup(), SortKey::Size, Order::Desc);
        assert_eq!(labels(&a), vec![("OVER 20 MB".into(), 4), ("2 TO 20 MB".into(), 15), ("UNDER 2 MB".into(), 5)]);
        let a = run(&mockup(), SortKey::Size, Order::Asc);
        assert_eq!(a.groups[0].label, "UNDER 2 MB");
    }

    #[test]
    fn filters() {
        let mut items = mockup();
        items[0].favourite = true;
        let f = |filter: Filter| arrange(&items, SortKey::Taken, Order::Desc, &filter, TODAY, 0, &HashSet::new()).order.len();
        assert_eq!(f(Filter::Videos), 4);
        assert_eq!(f(Filter::Favourites), 1);
        assert_eq!(f(Filter::Bucket(1)), 24);
        assert_eq!(f(Filter::Bucket(2)), 0);
    }

    #[test]
    fn hidden_bucket_left_out_of_all_but_not_its_album() {
        let mut items = mockup();
        for i in &mut items[..3] {
            i.bucket_id = 9;
        }
        let hidden: HashSet<i64> = [9].into();
        let all = arrange(&items, SortKey::Taken, Order::Desc, &Filter::All, TODAY, 0, &hidden);
        assert_eq!(all.order.len(), 21);
        let album = arrange(&items, SortKey::Taken, Order::Desc, &Filter::Bucket(9), TODAY, 0, &hidden);
        assert_eq!(album.order.len(), 3);
    }

    #[test]
    fn groups_cover_the_order_exactly() {
        for key in [SortKey::Taken, SortKey::Changed, SortKey::Name, SortKey::Size] {
            let a = run(&mockup(), key, Order::Desc);
            let mut next = 0;
            for g in &a.groups {
                assert_eq!(g.start, next);
                next += g.len;
            }
            assert_eq!(next as usize, a.order.len());
        }
    }

    #[test]
    #[ignore = "timing; run with --release -- --ignored"]
    fn arrange_20k_under_budget() {
        let base = mockup();
        let items: Vec<MediaItem> = (0..20_000)
            .map(|i| {
                let mut m = base[i % 24].clone();
                m.id = i as i64;
                m.taken_ms -= (i as i64) * 3_600_000;
                m.modified_ms = m.taken_ms;
                m.name = format!("IMG_{i:06}.jpg");
                m
            })
            .collect();
        for key in [SortKey::Taken, SortKey::Changed, SortKey::Name, SortKey::Size] {
            let t = std::time::Instant::now();
            let a = run(&items, key, Order::Desc);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            assert_eq!(a.order.len(), 20_000);
            assert!(ms < 30.0, "{key:?} took {ms:.1} ms");
        }
    }
}
