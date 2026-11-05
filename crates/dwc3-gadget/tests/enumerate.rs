//! `embassy-usb` (CDC-NCM + CDC-ACM) on the driver over the scripted DWC3
//! model: reset → GET_DESCRIPTOR → SET_ADDRESS → GET_DESCRIPTOR(config) →
//! SET_CONFIGURATION, then class requests and bulk OUT/IN data.

mod common;

use common::device::{parse_config, spawn_device, EpDesc, MAC, PID, VID};
use common::{get_descriptor, req, Bench};

const NCM_COMM: u8 = 0;
const NCM_DATA: u8 = 1;
const ACM_COMM: u8 = 2;

struct Enumerated {
    b: Bench,
    eps: Vec<EpDesc>,
}

impl Enumerated {
    fn ep(&self, iface: u8, alt: u8, attrs: u8, is_in: bool) -> EpDesc {
        *self
            .eps
            .iter()
            .find(|e| e.iface == iface && e.alt == alt && e.attrs == attrs && (e.addr >> 7 == 1) == is_in)
            .expect("endpoint")
    }
}

/// Bring the device to the Configured state the way a Linux host does.
fn enumerate() -> Enumerated {
    let mut b = Bench::new();
    spawn_device(&mut b);
    b.pump();
    assert!(b.hw.borrow().running, "device did not set RUN/STOP");
    b.bus_reset();

    let dev = b.control_in(get_descriptor(1, 0, 64)).unwrap();
    assert_eq!(dev.len(), 18);
    assert_eq!(&dev[..2], &[18, 1]);
    assert_eq!(dev[7], 64, "bMaxPacketSize0");
    assert_eq!(u16::from_le_bytes([dev[8], dev[9]]), VID);
    assert_eq!(u16::from_le_bytes([dev[10], dev[11]]), PID);

    b.control_out(req(0x00, 5, 7, 0, 0), &[]).unwrap();
    assert_eq!(b.hw.borrow().reg(0xc700) >> 3 & 0x7f, 7, "DCFG.DEVADDR");
    assert_eq!(b.hw.borrow().addr_at_status.last(), Some(&7), "address set before status stage");

    assert_eq!(b.control_in(get_descriptor(1, 0, 18)).unwrap(), dev);
    let head = b.control_in(get_descriptor(2, 0, 9)).unwrap();
    let total = u16::from_le_bytes([head[2], head[3]]);
    let cfg = b.control_in(get_descriptor(2, 0, total)).unwrap();
    assert_eq!(cfg.len(), total as usize);
    assert_eq!(cfg[4], 4, "bNumInterfaces");
    let (ifs, eps) = parse_config(&cfg);
    assert!(ifs.contains(&(NCM_COMM, 0, 0x02, 0x0d)), "{ifs:?}");
    assert!(ifs.contains(&(ACM_COMM, 0, 0x02, 0x02)), "{ifs:?}");
    for e in eps.iter().filter(|e| e.attrs == 2) {
        assert_eq!(e.mps, 512, "HS bulk wMaxPacketSize");
    }
    let lang = b.control_in(get_descriptor(3, 0, 255)).unwrap();
    assert_eq!(lang, [4, 3, 0x09, 0x04]);

    b.control_out(req(0x00, 9, 1, 0, 0), &[]).unwrap();
    b.assert_clean();
    Enumerated { b, eps }
}

#[test]
fn enumerates_reset_address_descriptors_configuration() {
    let e = enumerate();
    let hw = e.b.hw.borrow();
    let starts: Vec<_> = hw.cmds.iter().enumerate().filter(|(_, c)| c.1 == 9).collect();
    assert_eq!(starts.len(), 2, "DEPSTARTCFG(0) at connect + (2) at SET_CONFIGURATION");
    assert_eq!(*starts[1].1, (0, 9, 2));
    let first_user_cfg = hw.cmds.iter().position(|c| c.0 >= 2 && c.1 == 1).unwrap();
    assert!(starts[1].0 < first_user_cfg, "DEPSTARTCFG(2) before user DEPCFG");
    let dalepena = hw.reg(0xc720);
    let ncm_bulk = e.ep(NCM_DATA, 1, 2, false);
    assert_eq!(dalepena & (1 << ncm_bulk.phys()), 0, "NCM data alt 0 has no live endpoints");
    for ep in e.eps.iter().filter(|p| p.alt == 0) {
        assert_ne!(dalepena & (1 << ep.phys()), 0, "{ep:?} enabled");
    }
    // EP0 was re-configured to 64 bytes at connect-done.
    assert_eq!(hw.eps[0].mps, 64);
    assert_eq!(hw.eps[1].mps, 64);
}

#[test]
fn cdc_acm_line_coding_and_bulk_echo() {
    let mut e = enumerate();
    let coding = [0x00, 0xc2, 0x01, 0x00, 0, 0, 8]; // 115200 8N1
    e.b.control_out(req(0x21, 0x20, 0, ACM_COMM as u16, 7), &coding).unwrap();
    assert_eq!(e.b.control_in(req(0xa1, 0x21, 0, ACM_COMM as u16, 7)).unwrap(), coding);
    e.b.control_out(req(0x21, 0x22, 1, ACM_COMM as u16, 0), &[]).unwrap();

    let out = e.ep(3, 0, 2, false).phys();
    let inp = e.ep(3, 0, 2, true).phys();
    e.b.bulk_out(out, b"hello redox", 512, false).unwrap();
    assert_eq!(e.b.bulk_in(inp, 512).unwrap(), b"hello redox");
    let big: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
    e.b.bulk_out(out, &big, 512, false).unwrap();
    assert_eq!(e.b.bulk_in(inp, 512).unwrap(), big);
    e.b.assert_clean();
}

fn ntb(seq: u16, frame: &[u8]) -> Vec<u8> {
    let total = 28 + frame.len();
    let mut v = Vec::new();
    v.extend(0x484d_434eu32.to_le_bytes()); // "NCMH"
    v.extend(12u16.to_le_bytes());
    v.extend(seq.to_le_bytes());
    v.extend((total as u16).to_le_bytes());
    v.extend(12u16.to_le_bytes()); // NDP index
    v.extend(0x304d_434eu32.to_le_bytes()); // "NCM0"
    v.extend(16u16.to_le_bytes());
    v.extend(0u16.to_le_bytes());
    v.extend(28u16.to_le_bytes());
    v.extend((frame.len() as u16).to_le_bytes());
    v.extend([0u8; 4]);
    v.extend(frame);
    v
}

fn ntb_datagram(n: &[u8]) -> &[u8] {
    assert_eq!(&n[..4], b"NCMH");
    let ndp = u16::from_le_bytes([n[10], n[11]]) as usize;
    assert_eq!(&n[ndp..ndp + 4], b"NCM0");
    let idx = u16::from_le_bytes([n[ndp + 8], n[ndp + 9]]) as usize;
    let len = u16::from_le_bytes([n[ndp + 10], n[ndp + 11]]) as usize;
    &n[idx..idx + len]
}

#[test]
fn cdc_ncm_ntb_parameters_connection_and_frame_echo() {
    let mut e = enumerate();
    let p = e.b.control_in(req(0xa1, 0x80, 0, NCM_COMM as u16, 28)).unwrap();
    assert_eq!(p.len(), 28);
    assert_eq!(u16::from_le_bytes([p[2], p[3]]), 1, "16-bit NTB only");
    e.b.control_out(req(0x21, 0x86, 0, NCM_COMM as u16, 4), &2048u32.to_le_bytes()).unwrap();
    let mac = e.b.control_in(get_descriptor(3, 4, 255));
    assert!(mac.is_ok());

    e.b.control_out(req(0x01, 11, 1, NCM_DATA as u16, 0), &[]).unwrap(); // SET_INTERFACE alt 1
    let int = e.ep(NCM_COMM, 0, 3, true);
    assert_eq!(int.mps, 8);
    let notif = e.b.in_packet(int.phys()).unwrap();
    assert_eq!(notif, [0xa1, 0x00, 0x01, 0x00, NCM_DATA, 0, 0, 0], "NETWORK_CONNECTION");

    let out = e.ep(NCM_DATA, 1, 2, false).phys();
    let inp = e.ep(NCM_DATA, 1, 2, true).phys();
    let mut frame = vec![0xffu8; 6];
    frame.extend(MAC);
    frame.extend([0x08, 0x06]);
    frame.resize(60, 0x5a);
    e.b.bulk_out(out, &ntb(1, &frame), 512, true).unwrap();
    let back = e.b.bulk_in(inp, 512).unwrap();
    assert_eq!(ntb_datagram(&back), frame);

    let big: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
    e.b.bulk_out(out, &ntb(2, &big), 512, true).unwrap();
    let back = e.b.bulk_in(inp, 512).unwrap();
    assert_eq!(back.len(), 1028);
    assert_eq!(ntb_datagram(&back), big);
    e.b.assert_clean();
}
