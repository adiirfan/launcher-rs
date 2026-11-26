//! One photo or video, as Android's media list describes it, and the byte
//! format Java hands it over in (also used for the start-up snapshot).

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Image,
    Video,
    Gif,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaItem {
    pub id: i64,
    pub kind: Kind,
    pub w: u32,
    pub h: u32,
    pub orientation: u16,
    pub duration_ms: u64,
    pub size: u64,
    pub taken_ms: i64,
    pub modified_ms: i64,
    pub added_ms: i64,
    pub favourite: bool,
    pub bucket_id: i64,
    pub name: String,
    pub folder: String,
    pub mime: String,
    /// In the Bin: when it goes for good. 0 otherwise.
    pub expires_ms: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    BadHeader,
    Truncated,
    BadKind(u8),
    BadText,
}

impl MediaItem {
    /// When it was taken, or when it arrived on the phone if the file
    /// doesn't say (some downloads and screenshots).
    pub fn when_taken(&self) -> i64 {
        if self.taken_ms > 0 { self.taken_ms } else { self.added_ms }
    }
}

const MAGIC: &[u8; 4] = b"HGL2";

/// Little-endian: `HGL2`, u32 count, then per record: i64 id, u8 kind
/// (0 image, 1 video, 2 gif), u32 w, u32 h, u16 orientation, u64 duration_ms,
/// u64 size, i64 taken_ms, i64 modified_ms, i64 added_ms, u8 flags (bit 0
/// favourite), i64 bucket_id, i64 expires_ms, then name, folder and mime, each a u16 byte
/// length and UTF-8.
pub fn encode_records(items: &[MediaItem]) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + items.len() * 110);
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&(items.len() as u32).to_le_bytes());
    for i in items {
        b.extend_from_slice(&i.id.to_le_bytes());
        b.push(match i.kind {
            Kind::Image => 0,
            Kind::Video => 1,
            Kind::Gif => 2,
        });
        b.extend_from_slice(&i.w.to_le_bytes());
        b.extend_from_slice(&i.h.to_le_bytes());
        b.extend_from_slice(&i.orientation.to_le_bytes());
        b.extend_from_slice(&i.duration_ms.to_le_bytes());
        b.extend_from_slice(&i.size.to_le_bytes());
        b.extend_from_slice(&i.taken_ms.to_le_bytes());
        b.extend_from_slice(&i.modified_ms.to_le_bytes());
        b.extend_from_slice(&i.added_ms.to_le_bytes());
        b.push(i.favourite as u8);
        b.extend_from_slice(&i.bucket_id.to_le_bytes());
        b.extend_from_slice(&i.expires_ms.to_le_bytes());
        for s in [&i.name, &i.folder, &i.mime] {
            let s = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
            b.extend_from_slice(&(s.len() as u16).to_le_bytes());
            b.extend_from_slice(s);
        }
    }
    b
}

struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        if self.b.len() < n {
            return Err(ParseError::Truncated);
        }
        let (head, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ParseError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, ParseError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, ParseError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, ParseError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn text(&mut self) -> Result<String, ParseError> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| ParseError::BadText)
    }
}

pub fn parse_records(bytes: &[u8]) -> Result<Vec<MediaItem>, ParseError> {
    let mut r = Reader { b: bytes };
    if r.take(4).map_err(|_| ParseError::BadHeader)? != MAGIC {
        return Err(ParseError::BadHeader);
    }
    let count = r.u32()? as usize;
    // Never trust the count for the allocation: a record is at least 78 bytes.
    let mut out = Vec::with_capacity(count.min(bytes.len() / 78));
    for _ in 0..count {
        let id = r.i64()?;
        let kind = match r.u8()? {
            0 => Kind::Image,
            1 => Kind::Video,
            2 => Kind::Gif,
            k => return Err(ParseError::BadKind(k)),
        };
        out.push(MediaItem {
            id,
            kind,
            w: r.u32()?,
            h: r.u32()?,
            orientation: r.u16()?,
            duration_ms: r.u64()?,
            size: r.u64()?,
            taken_ms: r.i64()?,
            modified_ms: r.i64()?,
            added_ms: r.i64()?,
            favourite: r.u8()? & 1 != 0,
            bucket_id: r.i64()?,
            expires_ms: r.i64()?,
            name: r.text()?,
            folder: r.text()?,
            mime: r.text()?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, name: &str, folder: &str) -> MediaItem {
        MediaItem {
            id,
            kind: Kind::Video,
            w: 1920,
            h: 1080,
            orientation: 90,
            duration_ms: 64_000,
            size: 48_200_000,
            taken_ms: 1_795_000_000_000,
            modified_ms: 1_795_000_100_000,
            added_ms: 1_795_000_200_000,
            favourite: true,
            bucket_id: -1_739_773_001,
            name: name.into(),
            folder: folder.into(),
            mime: "video/mp4".into(),
            expires_ms: 0,
        }
    }

    #[test]
    fn round_trip() {
        let mut gif = item(2, "日本.gif", "");
        gif.kind = Kind::Gif;
        gif.favourite = false;
        // In the Bin: the day it goes for good.
        gif.expires_ms = 1_797_000_000_000;
        let items = vec![item(1, "VID_20261126_084210.mp4", "Camera"), gif];
        assert_eq!(parse_records(&encode_records(&items)), Ok(items));
    }

    #[test]
    fn empty_list_round_trips() {
        assert_eq!(parse_records(&encode_records(&[])), Ok(vec![]));
    }

    #[test]
    fn last_versions_list_is_refused() {
        let mut b = encode_records(&[item(1, "a.mp4", "x")]);
        b[3] = b'1';
        assert_eq!(parse_records(&b), Err(ParseError::BadHeader));
    }

    #[test]
    fn bad_header_is_error() {
        let mut b = encode_records(&[item(1, "a.mp4", "x")]);
        b[0] = b'X';
        assert_eq!(parse_records(&b), Err(ParseError::BadHeader));
    }

    #[test]
    fn truncated_record_is_error() {
        let mut b = encode_records(&[item(1, "a.mp4", "x")]);
        b.pop();
        assert_eq!(parse_records(&b), Err(ParseError::Truncated));
    }

    #[test]
    fn when_taken_falls_back_to_added() {
        let mut i = item(1, "a.jpg", "x");
        i.taken_ms = 0;
        i.added_ms = 5_000;
        assert_eq!(i.when_taken(), 5_000);
        i.taken_ms = 7;
        assert_eq!(i.when_taken(), 7);
    }
}
