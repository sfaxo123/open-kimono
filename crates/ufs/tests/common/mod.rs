//! Host test fixtures: shared DMA memory, the UFS device (UPIU responder), the UFSHCI register
//! model and a GPT disk builder. Not every test binary uses every item.
#![allow(dead_code)]

pub mod device;
pub mod disk;
pub mod luns;
pub mod model;

use std::cell::RefCell;
use std::rc::Rc;
use ufs_core::Dma;

/// Register window base used by all tests (the SDM845 `std` region).
pub const BASE: u64 = 0x01d8_4000;
/// Bus address of the test DMA region.
pub const DMA_PHYS: u64 = 0x9a00_0000;

/// A DMA region shared between the driver and the device model.
#[derive(Clone)]
pub struct SharedDma {
    pub mem: Rc<RefCell<Vec<u8>>>,
    pub phys: u64,
}

impl SharedDma {
    pub fn new(phys: u64, size: usize) -> Self {
        SharedDma {
            mem: Rc::new(RefCell::new(vec![0; size])),
            phys,
        }
    }

    /// Translate a bus address into a region offset (panics outside the region: a driver bug).
    pub fn off(&self, addr: u64, len: usize) -> usize {
        let off = addr
            .checked_sub(self.phys)
            .expect("DMA address below region") as usize;
        assert!(
            off + len <= self.mem.borrow().len(),
            "DMA access past region end"
        );
        off
    }

    pub fn get(&self, addr: u64, len: usize) -> Vec<u8> {
        let off = self.off(addr, len);
        self.mem.borrow()[off..off + len].to_vec()
    }

    pub fn put(&self, addr: u64, data: &[u8]) {
        let off = self.off(addr, data.len());
        self.mem.borrow_mut()[off..off + data.len()].copy_from_slice(data);
    }
}

impl Dma for SharedDma {
    fn phys(&self) -> u64 {
        self.phys
    }
    fn size(&self) -> usize {
        self.mem.borrow().len()
    }
    fn read(&mut self, off: usize, buf: &mut [u8]) {
        buf.copy_from_slice(&self.mem.borrow()[off..off + buf.len()]);
    }
    fn write(&mut self, off: usize, data: &[u8]) {
        self.mem.borrow_mut()[off..off + data.len()].copy_from_slice(data);
    }
}

/// Default test region: list + UCD + 64 KiB of data buffer.
pub fn dma() -> SharedDma {
    SharedDma::new(DMA_PHYS, 0x1000 + 0x10000)
}

/// Config with small poll budgets so negative tests stay fast and traces stay short.
pub fn config() -> ufs_core::Config {
    let mut c = ufs_core::Config::sdm845();
    c.polls = ufs_core::Polls {
        hce: 8,
        phy: 4,
        uic: 8,
        transfer: 16,
        clear: 8,
        device_init: 8,
    };
    c.nop_attempts = 3;
    c
}

/// A OnePlus-6-shaped device: LU0 user data (GPT), LU1 boot LU A (GPT), LU2 disabled,
/// LU3 enabled without a GPT. All 4 KiB blocks.
pub fn phone_device() -> device::UfsDevice {
    let lu0 = disk::gpt_image(
        4096,
        64,
        &[("ssd", 6, 7), ("misc", 8, 9), ("userdata", 10, 58)],
    );
    let lu1 = disk::gpt_image(4096, 16, &[("xbl_a", 6, 8), ("xbl_config_a", 9, 10)]);
    let mut boot = luns::Lu::new(12, lu1);
    boot.boot_id = 1;
    boot.write_protect = 1;
    let mut raw = vec![0u8; 8 * 4096];
    raw[4096] = 0x42;
    device::UfsDevice::new(vec![
        luns::Lu::new(12, lu0),
        boot,
        luns::Lu::disabled(),
        luns::Lu::new(12, raw),
    ])
}

/// Model + driver over the default DMA region.
pub fn rig(dev: device::UfsDevice) -> (model::UfsModel, SharedDma) {
    let dma = dma();
    (model::UfsModel::sdm845(dma.clone(), dev), dma)
}
