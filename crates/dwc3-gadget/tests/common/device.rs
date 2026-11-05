//! A composite CDC-NCM + CDC-ACM `embassy-usb` device running on the bench,
//! with echo tasks for both functions.

use super::{leak, Bench, Core};
use dwc3_gadget::Driver;
use embassy_usb::class::{cdc_acm, cdc_ncm};
use embassy_usb::Builder;
use std::cell::RefCell;

pub const VID: u16 = 0x1209;
pub const PID: u16 = 0x0d06;
pub const MAC: [u8; 6] = [0x02, 0x6f, 0x70, 0x36, 0x00, 0x01];

type Drv = Driver<'static, super::model::Regs, super::model::Dma>;

pub fn spawn_device(b: &mut Bench) {
    let core: &'static RefCell<Core> = b.core;
    let mut config = embassy_usb::Config::new(VID, PID);
    config.manufacturer = Some("redox-oneplus6");
    config.product = Some("OnePlus 6 gadget");
    config.serial_number = Some("K11");
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
    let ncm = cdc_ncm::CdcNcmClass::new(&mut builder, leak(cdc_ncm::State::new()), MAC, 512);
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

    let (mut tx, mut rx) = ncm.split();
    b.spawn(async move {
        let mut buf = [0u8; 1600];
        loop {
            if rx.wait_connection().await.is_err() {
                continue;
            }
            while let Ok(n) = rx.read_packet(&mut buf).await {
                if tx.write_packet(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    });
}

/// Endpoint descriptor found in a configuration descriptor.
#[derive(Clone, Copy, Debug)]
pub struct EpDesc {
    pub iface: u8,
    pub alt: u8,
    pub addr: u8,
    pub attrs: u8,
    pub mps: u16,
}

impl EpDesc {
    pub fn phys(&self) -> u8 {
        ((self.addr & 0x0f) << 1) | (self.addr >> 7)
    }
}

/// Interface `(number, alt, class, subclass)`.
pub type IfDesc = (u8, u8, u8, u8);

/// Interfaces and endpoints in a config descriptor.
pub fn parse_config(d: &[u8]) -> (Vec<IfDesc>, Vec<EpDesc>) {
    let (mut ifs, mut eps, mut i, mut cur) = (Vec::new(), Vec::new(), 0, (0, 0));
    while i + 2 <= d.len() && d[i] != 0 {
        match d[i + 1] {
            4 => {
                cur = (d[i + 2], d[i + 3]);
                ifs.push((d[i + 2], d[i + 3], d[i + 5], d[i + 6]));
            }
            5 => eps.push(EpDesc {
                iface: cur.0,
                alt: cur.1,
                addr: d[i + 2],
                attrs: d[i + 3],
                mps: u16::from_le_bytes([d[i + 4], d[i + 5]]),
            }),
            _ => {}
        }
        i += d[i] as usize;
    }
    (ifs, eps)
}
