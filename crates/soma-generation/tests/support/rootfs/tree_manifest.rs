use soma::OciDigest;

#[derive(Debug, Eq, PartialEq)]
pub struct TreeEntry {
    pub path: Vec<u8>,
    pub kind: u8,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u64,
    pub payload: Vec<u8>,
}

pub fn read_tree(store: &std::path::Path, digest: &OciDigest) -> Vec<TreeEntry> {
    let bytes = std::fs::read(store.join("v1/blobs/sha256").join(&digest.as_str()[7..])).unwrap();
    let mut decoder = Decoder {
        bytes: &bytes,
        offset: 0,
    };
    assert_eq!(decoder.take(8), b"SOMARFS\0");
    assert_eq!(decoder.u16(), 1);
    assert_eq!(decoder.u16(), 1);
    let count = decoder.u32();
    let mut entries = Vec::new();
    for _ in 0..count {
        let path = decoder.sized();
        let kind = decoder.take(1)[0];
        let mode = decoder.u32();
        let uid = decoder.u32();
        let gid = decoder.u32();
        let mtime = decoder.u64();
        assert_eq!(decoder.u32(), 0);
        let payload = match kind {
            2 => decoder.take(40).to_vec(),
            3 | 5 => decoder.sized(),
            1 | 4 => Vec::new(),
            _ => panic!("unknown tree kind"),
        };
        entries.push(TreeEntry {
            path,
            kind,
            mode,
            uid,
            gid,
            mtime,
            payload,
        });
    }
    assert_eq!(decoder.offset, bytes.len());
    entries
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Decoder<'_> {
    fn take(&mut self, count: usize) -> &[u8] {
        let end = self.offset.checked_add(count).unwrap();
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        value
    }

    fn sized(&mut self) -> Vec<u8> {
        let count = self.u32() as usize;
        self.take(count).to_vec()
    }

    fn u16(&mut self) -> u16 {
        u16::from_be_bytes(self.take(2).try_into().unwrap())
    }

    fn u32(&mut self) -> u32 {
        u32::from_be_bytes(self.take(4).try_into().unwrap())
    }

    fn u64(&mut self) -> u64 {
        u64::from_be_bytes(self.take(8).try_into().unwrap())
    }
}
