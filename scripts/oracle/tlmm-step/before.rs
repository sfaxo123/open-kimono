// SPDX-License-Identifier: MIT
//! SDM845 TLMM GPIO interrupt domain (normal, non-wake summary path).

use core::ptr::{read_volatile, write_volatile};

use super::{qcom_irq, InterruptController};
use crate::{
    dtb::{
        irqchip::{InterruptHandler, IrqCell, IrqDesc},
        translate_mmio_address, Fdt,
    },
    scheme::irq::irq_trigger,
    sync::CleanLockToken,
};
use syscall::{Error, Result, EINVAL};

const GPIO_COUNT: usize = 150;
// The 150 GPIO interrupt registers are split across three 4 MiB tiles.
// These run boundaries are hardware facts from SDM845 pin group definitions.
pub struct Tlmm {
    base: usize,
    virq_base: usize,
    enabled: [bool; GPIO_COUNT],
    probe_only: bool,
}

impl Tlmm {
    pub fn new() -> Self {
        Self {
            base: 0,
            virq_base: 0,
            enabled: [false; GPIO_COUNT],
            probe_only: false,
        }
    }
    fn read(&self, offset: usize) -> u32 {
        unsafe { read_volatile((self.base + offset) as *const u32) }
    }
    fn write(&self, offset: usize, value: u32) {
        unsafe { write_volatile((self.base + offset) as *mut u32, value) }
    }
}

impl InterruptHandler for Tlmm {
    fn irq_handler(&mut self, _summary_virq: u32, token: &mut CleanLockToken) {
        if self.probe_only {
            return;
        }
        for pin in 0..GPIO_COUNT {
            if !self.enabled[pin] {
                continue;
            }
            let status = qcom_irq::tlmm_pin_offset(pin as u32, 0xc).unwrap();
            if self.read(status) & 1 != 0 {
                self.write(status, 1);
                irq_trigger((self.virq_base + pin) as u16, token);
            }
        }
    }
}

impl InterruptController for Tlmm {
    fn irq_init(
        &mut self,
        fdt_opt: Option<&Fdt>,
        irq_desc: &mut [IrqDesc; 2048],
        ic_idx: usize,
        irq_idx: &mut usize,
    ) -> Result<()> {
        let fdt = fdt_opt.ok_or(Error::new(EINVAL))?;
        let node = fdt
            .all_compatible(&["qcom,sdm845-pinctrl"])
            .next()
            .ok_or(Error::new(EINVAL))?;
        let reg = node
            .reg()
            .and_then(|r| r.iter::<usize, usize>().next())
            .and_then(|r| r.ok())
            .ok_or(Error::new(EINVAL))?;
        if reg.len < 0xc00000 || *irq_idx + GPIO_COUNT > irq_desc.len() {
            return Err(Error::new(EINVAL));
        }
        let phys = translate_mmio_address(fdt, &node, &reg).ok_or(Error::new(EINVAL))?;
        self.base = crate::PHYS_OFFSET + phys;
        self.probe_only = node.raw_property("redox,irqchip-probe-only").is_some();
        self.virq_base = *irq_idx;
        for pin in 0..GPIO_COUNT {
            irq_desc[*irq_idx + pin].basic.ic_idx = ic_idx;
            irq_desc[*irq_idx + pin].basic.ic_irq = pin as u32;
        }
        *irq_idx += GPIO_COUNT;
        info!(
            "TLMM probe: phys={:#x} GPIOs={} virqs={}..{}",
            phys, GPIO_COUNT, self.virq_base, *irq_idx
        );
        Ok(())
    }
    fn irq_ack(&mut self) -> u32 {
        0
    }
    fn irq_eoi(&mut self, pin: u32) {
        if self.probe_only {
            return;
        }
        if let Some(offset) = qcom_irq::tlmm_pin_offset(pin, 0xc) {
            self.write(offset, 1);
        }
    }
    fn irq_enable(&mut self, pin: u32) {
        if self.probe_only {
            return;
        }
        let Some(offset) = qcom_irq::tlmm_pin_offset(pin, 8) else {
            return;
        };
        self.enabled[pin as usize] = true;
        self.write(offset, self.read(offset) | (3 << 5) | (1 << 4) | 1);
    }
    fn irq_disable(&mut self, pin: u32) {
        if self.probe_only {
            return;
        }
        let Some(offset) = qcom_irq::tlmm_pin_offset(pin, 8) else {
            return;
        };
        self.enabled[pin as usize] = false;
        self.write(offset, self.read(offset) & !1);
    }
    fn irq_configure(&mut self, irq_data: IrqCell) -> Result<()> {
        let IrqCell::L2(pin, flags) = irq_data else {
            return Err(Error::new(EINVAL));
        };
        let offset = qcom_irq::tlmm_pin_offset(pin, 8).ok_or(Error::new(EINVAL))?;
        let bits = qcom_irq::tlmm_type_bits(flags).ok_or(Error::new(EINVAL))?;
        if self.probe_only {
            return Ok(());
        }
        self.write(offset, (self.read(offset) & !(0b111 << 1)) | bits);
        Ok(())
    }
    fn irq_xlate(&self, irq_data: IrqCell) -> Result<usize> {
        let IrqCell::L2(pin, flags) = irq_data else {
            return Err(Error::new(EINVAL));
        };
        qcom_irq::tlmm_pin_offset(pin, 8)
            .filter(|_| qcom_irq::tlmm_type_bits(flags).is_some())
            .ok_or(Error::new(EINVAL))?;
        Ok(self.virq_base + pin as usize)
    }
    fn irq_to_virq(&self, pin: u32) -> Option<usize> {
        qcom_irq::tlmm_pin_offset(pin, 8).map(|_| self.virq_base + pin as usize)
    }
    fn irq_handler_eois_root(&self) -> bool {
        true
    }
    fn irq_wakeup_spec(&self, irq_data: IrqCell) -> Option<IrqCell> {
        let IrqCell::L2(gpio, flags) = irq_data else {
            return None;
        };
        qcom_irq::tlmm_wake_pin(gpio).map(|pin| IrqCell::L2(pin, flags))
    }
}
