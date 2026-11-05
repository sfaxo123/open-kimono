//! P5 stage 3 bench proof: a CDC-ACM-ONLY device carrying the
//! exact descriptor plan the daemon's `acm_shell` module pins
//! (VID/PID/strings/EP0 size), enumerated end to end over the
//! scripted DWC3 model, with the ACM control requests and bulk
//! echo the shell bridge depends on.
//!
//! This is the host-side half of stage 3's exit evidence: the
//! class layer the Redox glue registers is byte-identical to
//! what a host terminal will see. The usbgadgetd-side plan
//! constants live in `crates/usbgadgetd/src/acm_shell.rs`;
//! they are duplicated here as literals on purpose — this test
//! pins the WIRE values, and a drift on either side fails one
//! of the two suites.

mod common;

use common::device::parse_config;
use common::{get_descriptor, leak, req, Bench};
use dwc3_gadget::Driver;
use embassy_usb::class::cdc_acm;
use embassy_usb::Builder;
use std::cell::RefCell;

// The stage-3 descriptor plan (mirrors acm_shell.rs).
const VID: u16 = 0x1209;
const PID: u16 = 0x0d06;
const MANUFACTURER: &str = "redox-oneplus6";
const PRODUCT: &str = "OnePlus 6 gadget";
const SERIAL: &str = "K11";

const ACM_COMM: u8 = 0;
const ACM_DATA: u8 = 1;

type Drv = Driver<'static, common::model::Regs, common::model::Dma>;

fn spawn_acm_device(b: &mut Bench) {
    let core: &'static RefCell<common::Core> = b.core;
    let mut config = embassy_usb::Config::new(VID, PID);
    config.manufacturer = Some(MANUFACTURER);
    config.product = Some(PRODUCT);
    config.serial_number = Some(SERIAL);
    config.max_packet_size_0 = 64;
    config.composite_with_iads = true;
    config.device_class = 0xef;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;

    let mut builder: Builder<'static, Drv> = Builder::new(
        Driver::new(core),
        config,
        leak([0u8; 256]),
        leak([0u8; 64]),
        leak([0u8; 0]),
        leak([0u8; 128]),
    );
    let acm = cdc_acm::CdcAcmClass::new(&mut builder, leak(cdc_acm::State::new()), 512);
    let usb = leak(builder.build());
    b.spawn(async move { usb.run().await });

    let acm = leak(acm);
    b.spawn(async move {
        let mut buf = [0u8; 512];
        loop {
            acm.wait_connection().await;
            while let Ok(n) = acm.read_packet(&mut buf).await {
                if acm.write_packet(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    });
}

fn string_desc(b: &mut Bench, idx: u8) -> String {
    let d = b.control_in(get_descriptor(3, idx, 255)).unwrap();
    assert_eq!(d[1], 3, "string descriptor type");
    let units: Vec<u16> = d[2..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).unwrap()
}

fn enumerate_acm() -> (Bench, Vec<common::device::EpDesc>) {
    let mut b = Bench::new();
    spawn_acm_device(&mut b);
    b.pump();
    assert!(b.hw.borrow().running, "device did not set RUN/STOP");
    b.bus_reset();

    let dev = b.control_in(get_descriptor(1, 0, 64)).unwrap();
    assert_eq!(dev.len(), 18);
    assert_eq!(dev[7], 64, "bMaxPacketSize0");
    assert_eq!(u16::from_le_bytes([dev[8], dev[9]]), VID);
    assert_eq!(u16::from_le_bytes([dev[10], dev[11]]), PID);
    // String indices: 1 manufacturer, 2 product, 3 serial.
    assert_eq!(dev[14], 1);
    assert_eq!(dev[15], 2);
    assert_eq!(dev[16], 3);

    b.control_out(req(0x00, 5, 7, 0, 0), &[]).unwrap();
    assert_eq!(b.hw.borrow().reg(0xc700) >> 3 & 0x7f, 7, "DCFG.DEVADDR");

    assert_eq!(string_desc(&mut b, 1), MANUFACTURER);
    assert_eq!(string_desc(&mut b, 2), PRODUCT);
    assert_eq!(string_desc(&mut b, 3), SERIAL);

    let head = b.control_in(get_descriptor(2, 0, 9)).unwrap();
    let total = u16::from_le_bytes([head[2], head[3]]);
    let cfg = b.control_in(get_descriptor(2, 0, total)).unwrap();
    assert_eq!(cfg.len(), total as usize);
    assert_eq!(cfg[4], 2, "bNumInterfaces: ACM comm + data only");
    let (ifs, eps) = parse_config(&cfg);
    assert_eq!(ifs.len(), 2, "{ifs:?}");
    assert!(ifs.contains(&(ACM_COMM, 0, 0x02, 0x02)), "comm: {ifs:?}");
    assert!(ifs.contains(&(ACM_DATA, 0, 0x0a, 0x00)), "data: {ifs:?}");
    for e in eps.iter().filter(|e| e.attrs == 2) {
        assert_eq!(e.mps, 512, "HS bulk wMaxPacketSize");
    }

    b.control_out(req(0x00, 9, 1, 0, 0), &[]).unwrap();
    b.assert_clean();
    (b, eps)
}

#[test]
fn acm_only_device_enumerates_with_the_stage3_identity() {
    let (_b, _eps) = enumerate_acm();
}

#[test]
fn acm_line_coding_control_state_and_bulk_echo() {
    let (mut b, eps) = enumerate_acm();
    let ep = |alt: u8, attrs: u8, is_in: bool| {
        *eps.iter()
            .find(|e| e.iface == ACM_DATA && e.alt == alt && e.attrs == attrs && (e.addr >> 7 == 1) == is_in)
            .expect("endpoint")
    };

    // The class powers up at embassy's default (8000 8N1);
    // GET_LINE_CODING must say so before any SET.
    let default = b
        .control_in(req(0xa1, 0x21, 0, ACM_COMM as u16, 7))
        .unwrap();
    assert_eq!(default, [0x40, 0x1f, 0x00, 0x00, 0, 0, 8]);

    // SET then GET the console-standard 115200 8N1.
    let coding = [0x00, 0xc2, 0x01, 0x00, 0, 0, 8];
    b.control_out(req(0x21, 0x20, 0, ACM_COMM as u16, 7), &coding)
        .unwrap();
    assert_eq!(
        b.control_in(req(0xa1, 0x21, 0, ACM_COMM as u16, 7)).unwrap(),
        coding
    );

    // DTR asserted (a terminal opening the port).
    b.control_out(req(0x21, 0x22, 1, ACM_COMM as u16, 0), &[])
        .unwrap();

    let out = ep(0, 2, false).phys();
    let inp = ep(0, 2, true).phys();
    b.bulk_out(out, b"echo hi\n", 512, false).unwrap();
    assert_eq!(b.bulk_in(inp, 512).unwrap(), b"echo hi\n");
    let big: Vec<u8> = (0..300u32).map(|i| (i * 3 + 1) as u8).collect();
    b.bulk_out(out, &big, 512, false).unwrap();
    assert_eq!(b.bulk_in(inp, 512).unwrap(), big);
    b.assert_clean();
}
