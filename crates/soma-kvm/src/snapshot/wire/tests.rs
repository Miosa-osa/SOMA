use super::{Reader, WireError, Writer};

#[test]
fn round_trips_every_primitive_in_fixed_order() {
    let mut writer = Writer::default();
    writer.put_u8(0xab);
    writer.put_u16(0x0102);
    writer.put_u32(0x0304_0506);
    writer.put_u64(0x0708_090a_0b0c_0d0e);
    writer.put_i64(-2);
    writer.put_presence(true);
    writer.put_bytes(&[9, 8]);
    let bytes = writer.finish();
    assert_eq!(bytes.len(), 26);
    assert_eq!(bytes[1..3], [1, 2]);

    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.u8(), Ok(0xab));
    assert_eq!(reader.u16(), Ok(0x0102));
    assert_eq!(reader.u32(), Ok(0x0304_0506));
    assert_eq!(reader.u64(), Ok(0x0708_090a_0b0c_0d0e));
    assert_eq!(reader.i64(), Ok(-2));
    assert_eq!(reader.presence(), Ok(true));
    assert_eq!(reader.array::<2>(), Ok([9, 8]));
    assert_eq!(reader.finish(), Ok(()));
}

#[test]
fn rejects_short_input_trailing_bytes_and_bad_presence() {
    let mut reader = Reader::new(&[1]);
    assert_eq!(
        reader.u16(),
        Err(WireError::Truncated {
            needed: 2,
            available: 1
        })
    );
    assert_eq!(
        Reader::new(&[2]).presence(),
        Err(WireError::InvalidPresence(2))
    );
    assert_eq!(Reader::new(&[0]).finish(), Err(WireError::TrailingBytes(1)));
}

#[test]
fn checks_bounds_before_availability() {
    let bytes = [0xff, 0xff, 0xff, 0xff];
    let mut reader = Reader::new(&bytes);
    assert_eq!(
        reader.bounded_u32(16),
        Err(WireError::LengthExceedsBound {
            length: u64::from(u32::MAX),
            bound: 16
        })
    );
    let mut reader = Reader::new(&[0, 0, 0, 3, 1, 2]);
    assert_eq!(
        reader.bounded_u32(16),
        Err(WireError::Truncated {
            needed: 3,
            available: 2
        })
    );
    assert_eq!(
        Reader::new(&[0, 9]).count_u16(8),
        Err(WireError::LengthExceedsBound {
            length: 9,
            bound: 8
        })
    );
}
