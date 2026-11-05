use geni_i2c_core::{Bus, Error as BusError};
use rmi4_touch_core::{decode, Data1, Error, Rmi4, ADDRESS};
use std::collections::VecDeque;
struct Script {
    calls: VecDeque<(Vec<u8>, Vec<u8>)>,
}
impl Bus for Script {
    fn write(&mut self, _: u8, _: &[u8]) -> Result<(), BusError> {
        unreachable!()
    }
    fn write_read(&mut self, addr: u8, cmd: &[u8], out: &mut [u8]) -> Result<(), BusError> {
        assert_eq!(addr, ADDRESS);
        let (expected, value) = self.calls.pop_front().unwrap();
        assert_eq!(cmd, expected);
        assert_eq!(out.len(), value.len());
        out.copy_from_slice(&value);
        Ok(())
    }
}
fn descriptor(addr: u8, function: u8) -> (Vec<u8>, Vec<u8>) {
    (vec![addr], vec![0x10, 0x20, 0x30, 0x40, 1, function])
}
fn discovery() -> Script {
    Script {
        calls: VecDeque::from([
            descriptor(0xe9, 1),
            descriptor(0xe3, 0x12),
            (vec![0xdd], vec![0; 6]),
        ]),
    }
}
#[test]
fn scans_pdt_and_decodes_one_finger() {
    let mut bus = discovery();
    bus.calls.push_back((vec![0x40], vec![0, 2]));
    bus.calls
        .push_back((vec![0x41], vec![1, 0x37, 4, 0x2d, 3, 50, 2, 2]));
    let mut touch = Rmi4::discover(bus).unwrap();
    assert_eq!(touch.functions().f12.data, 0x40);
    touch.set_data1(Data1::checked(1, 1).unwrap());
    assert_eq!(
        touch.poll().unwrap(),
        vec![rmi4_touch_core::Contact {
            slot: 0,
            x: 1079,
            y: 813,
            pressure: 50
        }]
    );
    assert!(touch.into_bus().calls.is_empty());
}
#[test]
fn no_unverified_f12_read() {
    let mut touch = Rmi4::discover(discovery()).unwrap();
    assert_eq!(touch.poll(), Err(Error::UnconfiguredReport));
    assert!(touch.into_bus().calls.is_empty());
}
#[test]
fn missing_f12_and_bad_layout_fail() {
    let bus = Script {
        calls: VecDeque::from([descriptor(0xe9, 1), (vec![0xe3], vec![0; 6])]),
    };
    assert!(matches!(
        Rmi4::discover(bus),
        Err(Error::MissingFunction(0x12))
    ));
    assert_eq!(Data1::checked(250, 2), Err(Error::InvalidDescriptor));
}
#[test]
fn ignores_nonfinger_and_out_of_bounds() {
    let raw = [
        2, 4, 0, 5, 0, 1, 0, 0, 1, 0x38, 4, 5, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert!(decode(&raw).is_empty());
}
#[test]
fn reset_status_suppresses_report() {
    let mut bus = discovery();
    bus.calls.push_back((vec![0x40], vec![0x80, 0]));
    let mut touch = Rmi4::discover(bus).unwrap();
    touch.set_data1(Data1::checked(1, 1).unwrap());
    assert!(touch.poll().unwrap().is_empty());
    assert!(touch.into_bus().calls.is_empty());
}
