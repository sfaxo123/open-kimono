use geni_i2c_core::{Bus, Error, Geni};
use mmio_replay::{Event, Replayer, Trace, Width};
const BASE: u64 = 0xa90000;
fn r(t: &mut Trace, off: u64, v: u32) {
    t.push(Event::read(BASE + off, Width::W32, v as u64));
}
fn w(t: &mut Trace, off: u64, v: u32) {
    t.push(Event::write(BASE + off, Width::W32, v as u64));
}
fn phase(t: &mut Trace, len: u32, read: bool, hold: bool, value: u32) {
    r(t, 0x610, 0);
    w(t, 0x618, 0);
    w(t, if read { 0x270 } else { 0x26c }, len);
    w(
        t,
        0x600,
        ((if read { 2 } else { 1 }) << 27) | (0x20 << 9) | if hold { 4 } else { 0 },
    );
    r(t, 0x610, 0);
    if read {
        r(t, 0x804, 1);
        r(t, 0x780, value);
    } else {
        r(t, 0x800, 0);
        w(t, 0x700, value);
    }
    r(t, 0x610, 1);
    w(t, 0x618, 1);
}
#[test]
fn repeated_start_register_read_replays() {
    let mut t = Trace::new();
    phase(&mut t, 1, false, true, 0xe9);
    phase(&mut t, 2, true, false, 0x3412);
    let mut bus = Geni::new(Replayer::strict(t), BASE);
    let mut data = [0; 2];
    assert_eq!(bus.write_read(0x20, &[0xe9], &mut data), Ok(()));
    assert_eq!(data, [0x12, 0x34]);
    assert!(bus.into_io().finish().ok());
}
#[test]
fn invalid_transfer_makes_no_access() {
    let bus = Replayer::strict(Trace::new());
    let mut bus = Geni::new(bus, BASE);
    assert_eq!(bus.write(0x80, &[1]), Err(Error::InvalidTransfer));
    assert_eq!(
        bus.write_read(0x20, &[], &mut [0]),
        Err(Error::InvalidTransfer)
    );
    assert!(bus.into_io().finish().ok());
}
#[test]
fn controller_error_cancels() {
    let mut t = Trace::new();
    r(&mut t, 0x610, 0);
    w(&mut t, 0x618, 0);
    w(&mut t, 0x26c, 1);
    w(&mut t, 0x600, (1 << 27) | (0x20 << 9));
    r(&mut t, 0x610, 1 << 1);
    w(&mut t, 0x604, 1 << 2);
    let mut bus = Geni::new(Replayer::strict(t), BASE);
    assert_eq!(bus.write(0x20, &[1]), Err(Error::Controller(2)));
    assert!(bus.into_io().finish().ok());
}
#[test]
fn fifo_timeout_is_bounded_and_cancels() {
    let mut t = Trace::new();
    r(&mut t, 0x610, 0);
    w(&mut t, 0x618, 0);
    w(&mut t, 0x26c, 1);
    w(&mut t, 0x600, (1 << 27) | (0x20 << 9));
    for _ in 0..1024 {
        r(&mut t, 0x610, 0);
        r(&mut t, 0x800, 16);
    }
    w(&mut t, 0x604, 1 << 2);
    let mut bus = Geni::new(Replayer::strict(t), BASE);
    assert_eq!(bus.write(0x20, &[1]), Err(Error::Timeout));
    assert!(bus.into_io().finish().ok());
}
