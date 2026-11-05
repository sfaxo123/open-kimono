//! `dwc3-gadget` — Synopsys DWC3 USB device mode for the OnePlus 6 (SDM845 `usb_1`).
//!
//! A polled, single-threaded device-controller core plus the
//! [`embassy_usb_driver`] traits, so `embassy-usb` (CDC-NCM, CDC-ACM, …) runs
//! on top. Spec: `specs/dwc3-gadget.md`.
//!
//! Hardware access goes through two [`mmio_replay::Mmio`] windows:
//!
//! - `R`: registers, absolute addresses (core window + QSCRATCH glue);
//! - `D`: the DMA region (event ring, TRBs, packet buffers) addressed by its
//!   **bus** address ([`Config::dma_base`]), [`layout::DMA_SIZE`] bytes,
//!   uncached and accessed in program order.
//!
//! ```ignore
//! let core = RefCell::new(Dwc3::new(regs, dma, Config::sdm845_usb1(dma_bus, delay)));
//! core.borrow_mut().init()?;
//! let mut builder = embassy_usb::Builder::new(Driver::new(&core), config, ...);
//! ```
//!
//! Scope: USB 2.0 high/full speed, bulk + interrupt endpoints, one TRB per
//! packet. The QUSB2 PHY, clocks and GDSC are assumed firmware-initialised.

#![no_std]
#![forbid(unsafe_code)]

mod controller;
mod driver;
mod ep;
mod ep0;
mod events;
pub mod event;
pub mod layout;
pub mod regs;
pub mod trb;

pub use crate::controller::{BusEvent, Config, Dwc3, Error};
pub use driver::{Bus, ControlPipe, Driver, Endpoint};
pub use ep::{phys, Xfer};
pub use ep0::Stage as Ep0Stage;
